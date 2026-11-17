#!/usr/bin/env python3
"""Offline, isolated semantic-state export from the user's pinned client."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import zipfile

VERSION, PROTOCOL = '26.3', 777
SHA1 = 'e877b6a07acd633fb3bb475002175cec036e7b87'
MAX_BYTES = 12 * 1024 * 1024
MAIN = 'dev.horizonlab.switchstatefacts.BlockStateFactsExporter'


def digest(path, algorithm='sha256'):
    h = hashlib.new(algorithm)
    with path.open('rb') as f:
        for chunk in iter(lambda: f.read(65536), b''):
            h.update(chunk)
    return h.hexdigest()


def classpath(client, libraries, manifest):
    """Use this launcher's selected Maven versions, never every cached JAR."""
    if digest(client, 'sha1') != SHA1:
        raise ValueError('client JAR does not match the pinned SHA1')
    with zipfile.ZipFile(client) as z:
        v = json.loads(z.read('version.json'))
    if v.get('id') != VERSION or v.get('protocol_version') != PROTOCOL:
        raise ValueError('client version/protocol mismatch')
    spec = json.loads(manifest.read_text())
    if spec.get('version') != VERSION:
        raise ValueError('launcher metadata version mismatch')
    jars, versions = [client], {}
    for lib in spec['libraries']:
        # GUI ObjC bridge / native profiling are not loaded by registry bootstrap.
        if lib['name'].startswith(('ca.weblite:java-objc-bridge:',
                                  'com.mojang:jtracy-natives-')):
            continue
        parts = lib['name'].split(':')
        if len(parts) == 4:  # Platform native classifier; no native code needed.
            continue
        if len(parts) != 3 or any(not re.fullmatch(r'[A-Za-z0-9_.+\-]+', x)
                                  or x in ('.', '..') for x in parts):
            raise ValueError('unsupported Maven coordinate')
        group, artifact, version = parts
        key = (group, artifact)
        if key in versions and versions[key] != version:
            raise ValueError('conflicting dependency versions')
        versions[key] = version
        p = libraries.joinpath(*group.split('.'), artifact, version,
                               f'{artifact}-{version}.jar')
        if not p.is_file():
            raise ValueError(f'cached dependency missing: {p.name}')
        expected = lib.get('downloads', {}).get('artifact', {}).get('sha1')
        if not expected or digest(p, 'sha1') != expected:
            raise ValueError(f'cached dependency hash mismatch: {p.name}')
        if p not in jars:
            jars.append(p)
    return jars


def validate(path):
    if path.stat().st_size > MAX_BYTES:
        raise ValueError('state output exceeds cap')
    data = json.loads(path.read_bytes())
    meta, states = data['metadata'], data['states']
    if (meta.get('schema'), meta.get('version'), meta.get('protocol'),
            meta.get('jar_sha1')) != (2, VERSION, PROTOCOL, SHA1):
        raise ValueError('export metadata mismatch')
    if not states or len(states) != meta.get('state_count'):
        raise ValueError('state count mismatch')
    tuples, air, fluids = set(), [], 0
    for i, s in enumerate(states):
        if type(s['id']) is not int or s['id'] != i:
            raise ValueError('state IDs are not contiguous')
        props = s['properties']
        if not isinstance(props, dict) or any(not isinstance(k, str) or not k
                or not isinstance(v, str) for k, v in props.items()):
            raise ValueError('properties must be a name/value map')
        if not isinstance(s['block'], str) or not s['block'].startswith('minecraft:'):
            raise ValueError('invalid block name')
        key = (s['block'], tuple(sorted(props.items())))
        if key in tuples:
            raise ValueError('duplicate block/property tuple')
        tuples.add(key)
        if s['render_shape'] not in ('MODEL', 'INVISIBLE', 'ENTITYBLOCK_ANIMATED'):
            raise ValueError('invalid render shape')
        for name in ('is_air', 'fluid_empty', 'solid_render', 'can_occlude', 'full_collision'):
            if type(s[name]) is not bool:
                raise ValueError('invalid state flag')
        if type(s['map_color']) is not int or not -(2**31) <= s['map_color'] < 2**31:
            raise ValueError('invalid map color')
        if s['is_air']:
            air.append(s['block'])
        fluids += not s['fluid_empty']
    if set(air) != {'minecraft:air', 'minecraft:cave_air', 'minecraft:void_air'}:
        raise ValueError('expected air names missing; numeric IDs are not assumed')
    return dict(meta, air_count=len(air), fluid_count=fluids,
                sha256=digest(path), output_bytes=path.stat().st_size)


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    for name in ('client-jar', 'libraries-dir', 'java-home', 'output-dir'):
        p.add_argument('--' + name, required=True, type=Path)
    p.add_argument('--launcher-metadata', type=Path)
    p.add_argument('--preflight', action='store_true')
    a = p.parse_args(argv)
    try:
        client, libraries, java_home, output = (x.resolve() for x in
            (a.client_jar, a.libraries_dir, a.java_home, a.output_dir))
        manifest = a.launcher_metadata or libraries.parent / 'meta/net.minecraft/26.3.json'
        jars = classpath(client, libraries, manifest)
        version_env, versions = dict(os.environ), {}
        for key in ('JAVA_TOOL_OPTIONS', '_JAVA_OPTIONS', 'JDK_JAVA_OPTIONS'):
            version_env.pop(key, None)
        for binary in ('java', 'javac'):
            cmd = java_home / 'bin' / binary
            r = subprocess.run([str(cmd), '-version'], capture_output=True,
                               text=True, check=True, timeout=15, env=version_env)
            version = (r.stdout + r.stderr).strip()
            versions[binary] = version
            match = re.search(r'(?:javac\s+|version\s+")([0-9]+)', version)
            if not match or int(match[1]) < 25:
                raise ValueError(f'{binary} must support Java25')
        if a.preflight:
            print(json.dumps({'version': VERSION, 'protocol': PROTOCOL,
                              'client_sha1': SHA1, 'classpath_jars': len(jars),
                              'compile_release': 25, 'runtime': versions['java']}))
            return 0
        if output.exists() or a.output_dir.is_symlink():
            raise ValueError('output directory already exists; refusing overwrite')
        if shutil.disk_usage(output.parent if output.parent.exists() else Path.cwd()).free < 10*2**30 + MAX_BYTES:
            raise ValueError('10 GiB reserve would be threatened')
        output.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix='state-facts-') as scratch_name:
            scratch = Path(scratch_name)
            env = dict(os.environ)
            for key in ('HORIZON_HOME', 'HORIZON_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME'):
                env[key] = str(scratch / key)
                Path(env[key]).mkdir()
            env.update(HORIZON_LAUNCHER_NO_SELF_UPDATE='1', HORIZON_NO_AUDIO='1')
            (scratch / 'home').mkdir()
            for key in ('JAVA_TOOL_OPTIONS', '_JAVA_OPTIONS', 'JDK_JAVA_OPTIONS'):
                env.pop(key, None)
            classes = scratch / 'classes'; classes.mkdir()
            cp = os.pathsep.join(map(str, jars))
            src = Path(__file__).resolve().parent / 'src/main/java/dev/horizonlab/switchstatefacts/BlockStateFactsExporter.java'
            subprocess.run([str(java_home/'bin/javac'), '-J-Duser.home='+str(scratch/'home'),
                            '--release', '25', '-cp', cp,
                            '-d', str(classes), str(src)], cwd=scratch, env=env,
                           check=True, capture_output=True, text=True, timeout=60)
            bytecode = classes / 'dev/horizonlab/switchstatefacts/BlockStateFactsExporter.class'
            if int.from_bytes(bytecode.read_bytes()[6:8], 'big') != 69:
                raise ValueError('exporter bytecode is not Java25')
            staged = Path(tempfile.mkdtemp(prefix='.'+output.name+'-', dir=output.parent))
            try:
                r = subprocess.run([str(java_home/'bin/java'), '-Xmx512m',
                    '-Djava.awt.headless=true', '-Duser.home='+str(scratch/'home'),
                    '-cp', str(classes)+os.pathsep+cp, MAIN, str(staged/'state_facts.json')],
                    cwd=scratch, env=env, check=True, capture_output=True, text=True, timeout=60)
                meta = validate(staged/'state_facts.json')
                meta.update(compile_release=25, classpath_jars=len(jars))
                (staged/'metadata.json').write_text(json.dumps(meta, indent=2)+'\n')
                (staged/'export.log').write_text(r.stdout+r.stderr)
                if sum(f.stat().st_size for f in staged.rglob('*') if f.is_file()) > MAX_BYTES:
                    raise ValueError('total exporter output exceeds12MiB')
                if output.exists():
                    raise ValueError('output appeared during export')
                staged.rename(output)
            except BaseException:
                shutil.rmtree(staged)
                raise
        print(json.dumps(meta, indent=2))
        return 0
    except (OSError, ValueError, KeyError, zipfile.BadZipFile,
            subprocess.SubprocessError) as error:
        print('Export refused: '+str(error), file=__import__('sys').stderr)
        if isinstance(error, subprocess.CalledProcessError):
            print((error.stderr or '')[-4000:], file=__import__('sys').stderr)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
