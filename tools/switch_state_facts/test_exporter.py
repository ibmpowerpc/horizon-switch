import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import zipfile

spec = importlib.util.spec_from_file_location('switch_facts', Path(__file__).with_name('exporter.py'))
e = importlib.util.module_from_spec(spec)
spec.loader.exec_module(e)


def fixture():
    states = []
    for i, name in enumerate(('stone', 'air', 'cave_air', 'void_air', 'water')):
        states.append(dict(id=i, block='minecraft:'+name,
            properties={'level': '0'} if name == 'water' else {},
            is_air='air' in name, fluid_empty=name != 'water',
            render_shape='MODEL' if name == 'stone' else 'INVISIBLE',
            solid_render=name == 'stone', can_occlude=name == 'stone',
            full_collision=name == 'stone', map_color=0))
    return dict(metadata=dict(schema=2, version=e.VERSION, protocol=e.PROTOCOL,
        jar_sha1=e.SHA1, runtime='27', state_count=len(states)), states=states)


class ExporterTests(unittest.TestCase):
    def test_semantic_validation_and_rejection(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp)/'facts.json';good=fixture()
            path.write_text(json.dumps(good))
            result=e.validate(path)
            self.assertEqual((result['state_count'],result['air_count'],result['fluid_count']),(5,3,1))
            mutations = [lambda d:d['states'][1].update(id=0),
                lambda d:d['metadata'].update(protocol=776),
                lambda d:d['states'][0].update(properties='axis=y'),
                lambda d:d['states'][0].update(is_air=True)]
            for change in mutations:
                bad=copy.deepcopy(good);change(bad);path.write_text(json.dumps(bad))
                with self.assertRaises(ValueError):e.validate(path)

    def test_cached_classpath_pin_and_version_selection(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);client=root/'client.jar';client.write_bytes(b'bad')
            with self.assertRaises(ValueError):e.classpath(client,root,root/'absent')
            with zipfile.ZipFile(client, 'w') as jar:
                jar.writestr('version.json', json.dumps(dict(id=e.VERSION, protocol_version=e.PROTOCOL)))
            selected=root/'example/dependency/1/dependency-1.jar'
            selected.parent.mkdir(parents=True);selected.write_bytes(b'selected')
            other=root/'example/dependency/2/dependency-2.jar'
            other.parent.mkdir(parents=True);other.write_bytes(b'not selected')
            metadata=root/'metadata.json'
            library=dict(name='example:dependency:1', downloads=dict(artifact=dict(sha1=e.digest(selected,'sha1'))))
            metadata.write_text(json.dumps(dict(version=e.VERSION,libraries=[library])))
            with patch.object(e,'SHA1',e.digest(client,'sha1')):
                self.assertEqual(e.classpath(client,root,metadata),[client,selected])
                selected.write_bytes(b'corrupted')
                with self.assertRaises(ValueError):e.classpath(client,root,metadata)

    def test_failed_validation_cleans_staging_and_uses_scratch(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);out=root/'out';sentinel=root/'unrelated';sentinel.write_text('keep')
            def run(args, **kwargs):
                if '-version' in args:
                    text='javac 27' if 'javac' in args[0] else 'openjdk version "27"'
                    return subprocess.CompletedProcess(args,0,text,'')
                cwd=Path(kwargs['cwd'])
                self.assertNotEqual(cwd,root)
                self.assertTrue(cwd.is_dir())
                self.assertEqual(kwargs['env']['HORIZON_NO_AUDIO'],'1')
                if '-d' in args:
                    self.assertIn('--release',args)
                    self.assertTrue(any(a.startswith('-J-Duser.home=') for a in args))
                    p=Path(args[args.index('-d')+1])/'dev/horizonlab/switchstatefacts/BlockStateFactsExporter.class'
                    p.parent.mkdir(parents=True);p.write_bytes(b'\xca\xfe\xba\xbe\x00\x00\x00\x45')
                else:
                    self.assertTrue(any(a.startswith('-Duser.home=') for a in args))
                    Path(args[-1]).write_text('{broken')
                return subprocess.CompletedProcess(args,0,'','')
            with patch.object(e,'classpath',return_value=[]),patch.object(e.subprocess,'run',side_effect=run),patch.object(e.shutil,'disk_usage',return_value=type('Disk',(),{'free':20*2**30})()):
                status=e.main(['--client-jar',str(root/'jar'),'--libraries-dir',str(root/'libs'),
                    '--java-home',str(root/'jdk'),'--output-dir',str(out)])
            self.assertEqual(status,1)
            self.assertEqual([p.name for p in root.iterdir()],['unrelated'])
            self.assertEqual(sentinel.read_text(),'keep')


if __name__=='__main__':unittest.main()
