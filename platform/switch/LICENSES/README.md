# Switch probe dependency notices

The encrypted probe links the installed devkitPro mbedTLS 2.28.10 library.
The installed mbedtls/rsa.h records: Copyright The Mbed TLS Contributors;
SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later. The Apache-2.0 text
is included here (standard text from the already-cached dependency license).
mbedTLS source has not been modified or vendored by this probe.

The probe also links libnx and the SDK runtime, and the Rust archive includes
Rust core/compiler_builtins and miniz_oxide/adler2. These dependencies retain
their own notices/licenses. OpenSSL is used by host verification only, not
linked into the Switch NRO. Generated test keys remain in ignored temporary
folders and are removed after fixtures.
