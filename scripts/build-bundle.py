#!/usr/bin/env python3
"""Build an immutable reviewed source snapshot into a fresh private candidate."""
from pathlib import Path
import argparse
import hashlib
import json
import os
import plistlib
import re
import subprocess
import sys
import tempfile
import tomllib
import privacy_guard as guard

project = Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('profile', nargs='?', choices=['release', 'debug'], default='release')
parser.add_argument('--private-rules', type=Path)
args = parser.parse_args()
rules = guard.private_rules(args.private_rules)
files = guard.manifest_files(project, forbidden=rules)
version = tomllib.loads(files['Cargo.toml'].decode())['package']['version']
source_hashes = {name: hashlib.sha256(data).hexdigest() for name, data in files.items()}
source_digest = hashlib.sha256(json.dumps(source_hashes, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
base = project / 'dist/candidates'
base.mkdir(parents=True, exist_ok=True)
guard.checked_root(base)
candidate = Path(tempfile.mkdtemp(prefix=version + '-', dir=base))
source = candidate / 'source'
source.mkdir(mode=0o700)
for name, data in files.items():
    path = source / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    path.chmod(0o555 if name.endswith('.sh') else 0o444)
for folder in sorted([p for p in source.rglob('*') if p.is_dir()],key=lambda p:len(p.parts),reverse=True):
    folder.chmod(0o555)
source.chmod(0o555)


def checked_snapshot():
    guard.exact_source_tree(source, files)
    current = guard.manifest_files(source, require_index=False, forbidden=rules)
    if current != files:
        raise SystemExit('Frozen source changed; candidate cannot be published')


def execute(arguments, **options):
    return subprocess.run([str(a) for a in arguments], check=True, **options)


try:
    execute(['sh', 'scripts/bootstrap-signing.sh'], cwd=project)
    tool_root = (project/'.tools').resolve()
    signing_env = {'HOME':str(Path.home()), 'PATH':'/usr/bin:/bin:/usr/sbin:/sbin',
                   'LANG':'en_US.UTF-8', 'RUST_EXPLORER_TOOLCHAIN_ROOT':str(tool_root),
                   'PYTHONDONTWRITEBYTECODE':'1', 'PYTHONNOUSERSITE':'1'}
    preparation = execute([sys.executable, source/'scripts/sign-local.py', 'prepare', project/'dist/Rust Explorer.app'], env=signing_env, capture_output=True, text=True)
    identity = json.loads(preparation.stdout)
    guard.certificate_requirement(identity)
    reviews = project/'dist/reviews'
    reviews.mkdir(mode=0o700, exist_ok=True)
    guard.checked_root(reviews)
    trusted_path = reviews/(candidate.name+'.json')
    expected = {'version':version,'source_digest':source_digest,'source_files':source_hashes,
                'certificate_sha1':identity['certificate_sha1'],'designated_requirement':identity['designated_requirement']}
    with trusted_path.open('x') as output:
        json.dump(expected, output, sort_keys=True)
    trusted_path.chmod(0o600)
    cargo_home = tool_root/'cargo'
    for root in [cargo_home, *source.parents]:
        for relative in (['config','config.toml'] if root==cargo_home else ['.cargo/config','.cargo/config.toml']):
            if (root/relative).exists():
                raise SystemExit('Unreviewed Cargo configuration must be isolated before building')
    env = {'HOME':str(Path.home()),'PATH':str(tool_root/'cargo/bin')+':/usr/bin:/bin:/usr/sbin:/sbin',
           'LANG':'en_US.UTF-8','LC_ALL':'en_US.UTF-8','RUST_EXPLORER_TOOLCHAIN_ROOT':str(tool_root),
           'RUST_EXPLORER_PYTHON':'/usr/bin/python3','PYTHONNOUSERSITE':'1','CARGO_TARGET_DIR':str(candidate/'target'),
           'MACOSX_DEPLOYMENT_TARGET':'14.0','RUSTUP_TOOLCHAIN':'1.99.1','RUSTUP_AUTO_INSTALL':'0',
           'PYTHONDONTWRITEBYTECODE':'1','TMPDIR':str(candidate/'tmp')}
    (candidate/'tmp').mkdir(mode=0o700)
    (candidate/'target').mkdir(mode=0o700)
    sdk = execute(['/usr/bin/xcrun','--sdk','macosx','--show-sdk-path'],capture_output=True,text=True).stdout.strip()
    env['SDKROOT']=sdk
    # Offline Cargo alone does not block build-script networking. The sandbox
    # also denies home data outside reviewed inputs/toolchain and private output.
    q = lambda value: json.dumps(value, ensure_ascii=False)
    allowed_reads = [source,tool_root,candidate/'target',candidate/'tmp',Path(sdk)]
    allowed_writes = [candidate/'target',candidate/'tmp']
    read_exceptions = ' '.join('(require-not (subpath '+q(str(path))+'))' for path in allowed_reads)
    write_exceptions = ' '.join('(require-not (subpath '+q(str(path))+'))' for path in allowed_writes)
    # Cargo's cache locks/accounting are the only permitted shared cache writes.
    for name in ['.package-cache','.package-cache-mutate','.global-cache','.global-cache-wal','.global-cache-shm']:
        write_exceptions += ' (require-not (literal '+q(str(cargo_home/name))+'))'
    policy = '(version 1)(allow default)(deny network*)'
    policy += '(deny file-write* (subpath '+q(str(source))+'))'
    policy += '(deny file-read-data (require-all (subpath '+q(str(Path.home()))+') '+read_exceptions+'))'
    policy += '(deny file-write* (require-all (subpath '+q(str(Path.home()))+') '+write_exceptions+'))'
    # Deny registry credentials even if a future cache contains them.
    for name in ['credentials','credentials.toml']:
        policy += '(deny file-read* (literal '+q(str(cargo_home/name))+'))'
    command = ['/usr/bin/sandbox-exec','-p',policy,'sh',str(source/'scripts/cargo.sh'),'build','--locked','--offline','--bin','rust-explorer','--target','aarch64-apple-darwin','--message-format=json-render-diagnostics']
    if args.profile == 'release':
        command.append('--release')
    artifacts = []
    with (candidate/'build.private.log').open('w') as errors:
        result = execute(command, cwd=source, env=env, stdout=subprocess.PIPE, stderr=errors, text=True)
    for line in result.stdout.splitlines():
        event = json.loads(line)
        if event.get('reason') == 'compiler-artifact' and event.get('target', {}).get('name') == 'rust-explorer' and event.get('executable'):
            artifacts.append(Path(event['executable']))
    if len(artifacts) != 1 or not artifacts[0].is_relative_to(candidate/'target'):
        raise SystemExit('Cargo did not produce the expected controlled executable')
    checked_snapshot()
    # Ensure signing helpers and inputs were not edited while compilation ran.
    if guard.manifest_files(project, forbidden=rules) != files:
        raise SystemExit('Reviewed inputs changed during build')
    binary_data = guard.read_file(candidate/'target', artifacts[0].relative_to(candidate/'target').as_posix())
    guard.audit_bytes(binary_data, '<executable>', rules)
    architecture = execute(['/usr/bin/lipo', '-archs', artifacts[0]], capture_output=True, text=True).stdout.strip()
    if architecture != 'arm64':
        raise SystemExit('Unexpected executable architecture')
    commands = execute(['/usr/bin/otool', '-l', artifacts[0]], capture_output=True, text=True).stdout
    if 'cmd LC_RPATH' in commands or not re.search(r'\bminos 14\.0\b', commands):
        raise SystemExit('Unexpected runtime search path or deployment target')
    linked = execute(['/usr/bin/otool', '-L', artifacts[0]], capture_output=True, text=True).stdout.splitlines()[1:]
    if any(not line.strip().startswith(('/usr/lib/', '/System/Library/Frameworks/')) for line in linked):
        raise SystemExit('Non-system dynamic library dependency')
    app = candidate/'Rust Explorer.app'
    (app/'Contents/MacOS').mkdir(parents=True)
    (app/'Contents/Resources').mkdir()
    binary = app/'Contents/MacOS/Rust Explorer'
    binary.write_bytes(binary_data)
    binary.chmod(0o755)
    info = files['resources/Info.plist']
    declaration = plistlib.loads(info)
    if declaration.get('CFBundleIconFile') != 'ExplorerFolder.icns' or declaration.get('CFBundleShortVersionString') != version or declaration.get('CFBundleIdentifier') != 'local.rust-explorer.app':
        raise SystemExit('Unexpected bundle declarations')
    (app/'Contents/Info.plist').write_bytes(info)
    (app/'Contents/Resources/ExplorerFolder.icns').write_bytes(files['resources/AppIcon.icns'])
    execute([sys.executable, source/'scripts/verify-signing-tool.py'], env=signing_env)
    execute([sys.executable, source/'scripts/sign-local.py', 'sign', app], env=signing_env)
    execute(['codesign','--verify','--deep','--strict','-R','='+expected['designated_requirement'],app])
    checked_snapshot()
    bundled = guard.bundle_files(app, rules)
    record = {'version': version, 'profile': args.profile, 'source_digest': source_digest,
              'source_files': source_hashes, 'app_files': {name: hashlib.sha256(data).hexdigest() for name, data in bundled.items()},
              'architecture': architecture, 'minimum_macos': '14.0', 'system_libraries_only': True,
              'scoped_privacy_checks_passed': True}
    (candidate/'build-record.json').write_text(json.dumps(record, sort_keys=True, indent=2)+'\n')
    expected['app_files'] = record['app_files']
    expected['build_completed'] = True
    with tempfile.NamedTemporaryFile(mode='w', dir=reviews, delete=False) as output:
        json.dump(expected, output, sort_keys=True)
        final_record = Path(output.name)
    final_record.chmod(0o600)
    os.replace(final_record, trusted_path)
    print(json.dumps({'candidate':str(candidate.relative_to(project)),'expected_inputs':str(trusted_path.relative_to(project))}))
except BaseException:
    (candidate/'REJECTED').write_text('Build did not complete. Do not publish this candidate.\n')
    raise
