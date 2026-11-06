#!/usr/bin/env python3
"""Check update identity with native codesign and inert command-line fixtures."""
from pathlib import Path
import hashlib
import importlib.util
import json
import os
import plistlib
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('local_sign', ROOT/'scripts/sign-local.py')
signer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(signer)
os.umask(0o077)


def command(args, success=True):
    result = subprocess.run([str(a) for a in args], capture_output=True, text=True)
    assert (result.returncode == 0) == success, (args, result.stderr)
    return result.stdout + result.stderr


def app_at(directory, version):
    app = directory/'Rust Explorer.app'
    (app/'Contents/MacOS').mkdir(parents=True)
    (app/'Contents/Resources').mkdir()
    (app/'Contents/Resources/value.txt').write_text(version)
    source = directory/'probe.c'
    source.write_text('#include <stdio.h>\nint main(void) { puts("'+version+'"); return 0; }\n')
    command(['/usr/bin/clang', '-o', app/'Contents/MacOS/Rust Explorer', source])
    (app/'Contents/Info.plist').write_bytes(plistlib.dumps(dict(
        CFBundleIdentifier=signer.BUNDLE_ID, CFBundleExecutable='Rust Explorer',
        CFBundleName='Rust Explorer', CFBundlePackageType='APPL', CFBundleVersion=version)))
    return app


def must_refuse(function):
    try:
        function()
    except RuntimeError:
        return
    raise AssertionError('expected identity-loss/tampering rejection')


(ROOT/'.test-data').mkdir(exist_ok=True)
with tempfile.TemporaryDirectory(prefix='signing-test-', dir=ROOT/'.test-data') as temporary:
    root = Path(temporary)
    first, second, foreign = [app_at(root/name, str(i)) for i, name in enumerate(['one','two','foreign'], 1)]
    identity = root/'private-identity'
    signer.identity(identity, first)
    one = signer.sign(first, identity)
    second_result = signer.sign(second, identity)
    assert one['designated_requirement'] == second_result['designated_requirement']
    assert command([first/'Contents/MacOS/Rust Explorer']).strip() == '1'
    assert command([second/'Contents/MacOS/Rust Explorer']).strip() == '2'
    command(['/usr/bin/codesign','--verify','--strict','-R','='+one['designated_requirement'],second])
    first_sha = hashlib.sha256((first/'Contents/MacOS/Rust Explorer').read_bytes()).hexdigest()
    second_sha = hashlib.sha256((second/'Contents/MacOS/Rust Explorer').read_bytes()).hexdigest()
    assert first_sha != second_sha
    signer.identity(root/'foreign-identity', foreign)
    signer.sign(foreign, root/'foreign-identity')
    command(['/usr/bin/codesign','--verify','--strict','-R','='+one['designated_requirement'],foreign], success=False)
    (second/'Contents/Resources/value.txt').write_text('tampered')
    command(['/usr/bin/codesign','--verify','--strict',second], success=False)
    saved = root/'saved-identity'
    identity.rename(saved)
    must_refuse(lambda: signer.identity(identity, first))
    assert not identity.exists()
    saved.rename(identity)
    os.chmod(identity/'identity.key', 0o644)
    must_refuse(lambda: signer.identity(identity, first))
    os.chmod(identity/'identity.key', 0o600)
    meta = identity/'identity.json'
    original = meta.read_text()
    meta.write_text(json.dumps(dict(certificate_sha1='0'*40)))
    must_refuse(lambda: signer.identity(identity, first))
    meta.write_text(original)
    assert signer.sign(first, identity)['certificate_sha1'] == one['certificate_sha1']
    report = dict(
        native_cross_version_requirement_passed=True, distinct_binaries=True,
        both_fixture_executables_ran=True, foreign_key_rejected=True,
        tampered_bundle_rejected=True, missing_identity_refuses_rotation=True,
        insecure_key_permissions_rejected=True, changed_certificate_pin_rejected=True,
        system_keychain_changed=False, system_trust_changed=False, tcc_permissions_changed=False,
        scope='Native code identity and execution checked; actual TCC grants were neither reset nor modified')
    print(json.dumps(report, indent=2))
