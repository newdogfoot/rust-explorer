#!/usr/bin/env python3
"""Stable, personal-use signing. No keychain/trust/TCC changes or network signing."""
from pathlib import Path
import argparse
import hashlib
import json
import os
import plistlib
import re
import stat
import subprocess
import tempfile

PROJECT = Path(__file__).resolve().parent.parent
TOOLS = Path(os.environ.get('RUST_EXPLORER_TOOLCHAIN_ROOT', PROJECT/'.tools')).resolve()
SIGNER = TOOLS / 'signing/bin/rcodesign'
DEFAULT_IDENTITY = Path.home() / 'Library/Application Support/Rust Explorer Signing'
BUNDLE_ID = 'local.rust-explorer.app'


def run(arguments, **options):
    return subprocess.run([str(a) for a in arguments], check=True, capture_output=True, **options)


def private_file(path):
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
        raise RuntimeError(f'Expected an owner-only regular signing file: {path}')


def fingerprint(certificate):
    der = run(['/usr/bin/openssl', 'x509', '-in', certificate, '-outform', 'DER']).stdout
    return hashlib.sha1(der).hexdigest()


def requirement(app):
    if not (app / 'Contents/MacOS/Rust Explorer').exists():
        return None
    output = run(['/usr/bin/codesign', '-d', '-r-', app])
    text = (output.stdout + output.stderr).decode()
    match = re.search(r'designated => (.+)', text)
    return match.group(1) if match else None


def identity(directory, previous_app=None):
    old_requirement = requirement(previous_app) if previous_app else None
    key, cert, metadata = [directory / name for name in ('identity.key', 'identity.crt', 'identity.json')]
    if not directory.exists():
        # Losing an existing certificate must never silently rotate identity.
        if old_requirement and 'certificate leaf' in old_requirement:
            raise RuntimeError('The existing app uses a persistent certificate but its signing folder is missing. Restore the signing folder before updating.')
        directory.mkdir(mode=0o700, parents=True)
    info = directory.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
        raise RuntimeError('Signing directory must be owned by this user with mode 0700, and must not be a symlink.')
    existing = [path.exists() or path.is_symlink() for path in (key, cert, metadata)]
    if any(existing) and not all(existing):
        raise RuntimeError('Incomplete signing identity; restore its original files. Refusing to generate a replacement.')
    if not any(existing):
        if old_requirement and 'certificate leaf' in old_requirement:
            raise RuntimeError('Existing signing identity is missing; refusing to rotate it.')
        with tempfile.TemporaryDirectory(prefix='.create-', dir=directory) as temporary:
            stage = Path(temporary)
            config = stage / 'certificate.cnf'
            config.write_text('''[req]
prompt = no
distinguished_name = subject
x509_extensions = extensions
[subject]
CN = Rust Explorer Local Signing
O = Personal local builds
[extensions]
basicConstraints = critical,CA:false
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid
''')
            # Private key bytes never appear in arguments, logs or stdout.
            run(['/usr/bin/openssl', 'req', '-new', '-x509', '-newkey', 'rsa:3072', '-nodes',
                 '-sha256', '-days', '3650', '-config', config,
                 '-keyout', stage/'identity.key', '-out', stage/'identity.crt'])
            # rcodesign consumes PKCS#8; LibreSSL req can output PKCS#1.
            run(['/usr/bin/openssl', 'pkcs8', '-topk8', '-nocrypt', '-in', stage/'identity.key', '-out', stage/'pkcs8.key'])
            os.replace(stage/'pkcs8.key', stage/'identity.key')
            pin = fingerprint(stage/'identity.crt')
            (stage/'identity.json').write_text(json.dumps(dict(format=1, certificate_sha1=pin, purpose='personal-local-builds'), indent=2)+'\n')
            for name in ('identity.key', 'identity.crt', 'identity.json'):
                os.chmod(stage/name, 0o600)
                # No replacement of an identity created by a concurrent build.
                os.link(stage/name, directory/name)
    for path in (key, cert, metadata):
        private_file(path)
    pin = fingerprint(cert)
    if json.loads(metadata.read_text())['certificate_sha1'] != pin:
        raise RuntimeError('Certificate differs from saved identity. Restore the original certificate.')
    run(['/usr/bin/openssl', 'x509', '-in', cert, '-checkend', '0', '-noout'])
    key_public = run(['/usr/bin/openssl', 'pkey', '-in', key, '-pubout']).stdout
    cert_public = run(['/usr/bin/openssl', 'x509', '-in', cert, '-pubkey', '-noout']).stdout
    if key_public != cert_public:
        raise RuntimeError('Signing key and certificate do not match.')
    expected = f'identifier "{BUNDLE_ID}" and certificate leaf = H"{pin}"'
    if old_requirement and 'certificate leaf' in old_requirement:
        # Preserve the exact certificate that already owns user consent.
        old_pin = re.search(r'certificate leaf = H"([0-9a-fA-F]+)"', old_requirement)
        if not old_pin or old_pin.group(1).lower() != pin:
            raise RuntimeError('This app was signed by another identity. Refusing to replace it silently.')
    return key, cert, expected


def sign(app, directory):
    import sys
    run([sys.executable, PROJECT/'scripts/verify-signing-tool.py'])
    key, cert, expected = identity(directory, app)
    info = plistlib.loads((app/'Contents/Info.plist').read_bytes())
    if info.get('CFBundleIdentifier') != BUNDLE_ID:
        raise RuntimeError('Unexpected application identity.')
    with tempfile.TemporaryDirectory(prefix='.requirement-', dir=directory) as temporary:
        compiled = Path(temporary)/'designated.bin'
        run(['/usr/bin/csreq', '-r', '='+expected, '-b', compiled])
        run(['/usr/bin/sandbox-exec', '-p', '(version 1)(allow default)(deny network*)',
             SIGNER, '--config-file', '/dev/null', 'sign', '--pem-file', key, '--pem-file', cert,
             '--timestamp-url', 'none', '--code-requirements-file', compiled, app])
    run(['/usr/bin/codesign', '--verify', '--deep', '--strict', '-R', '='+expected, app])
    actual = requirement(app)
    if actual != expected:
        raise RuntimeError(f'Unexpected designated requirement: {actual}')
    return dict(certificate_sha1=fingerprint(cert), designated_requirement=actual,
                signature_verified=True,
                apple_notarized=False, system_trust_changed=False)


if __name__ == '__main__':
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['prepare', 'sign'])
    parser.add_argument('app', type=Path)
    parser.add_argument('--identity-dir', type=Path, default=DEFAULT_IDENTITY)
    args = parser.parse_args()
    if args.action == 'prepare':
        _, cert, expected = identity(args.identity_dir, args.app)
        print(json.dumps(dict(certificate_sha1=fingerprint(cert), designated_requirement=expected)))
    else:
        print(json.dumps(sign(args.app, args.identity_dir), indent=2))
