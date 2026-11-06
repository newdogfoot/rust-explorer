#!/usr/bin/env python3
"""Package only one successful frozen build. No fallback to an older release."""
from pathlib import Path
import argparse
import hashlib
import json
import os
import re
import subprocess
import tempfile
import plistlib
import tomllib
import stat
import privacy_guard as guard

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--candidate', type=Path, required=True)
parser.add_argument('--private-rules', type=Path)
parser.add_argument('--expected-inputs', type=Path, required=True)
args = parser.parse_args()
rules = guard.private_rules(args.private_rules)
candidate = guard.checked_root(args.candidate)
if (candidate/'REJECTED').exists() or (candidate/'publish').exists():
    raise SystemExit('Candidate is rejected or already packaged; create a new candidate')
trusted = guard.checked_root(args.expected_inputs.parent)
if candidate==trusted or candidate in trusted.parents:
    raise SystemExit('Expected inputs must be held independently outside the candidate')
info=args.expected_inputs.lstat()
if not stat.S_ISREG(info.st_mode) or info.st_uid!=os.getuid() or info.st_mode&0o077:
    raise SystemExit('Expected input record must be an owner-only regular file')
expected=json.loads(guard.read_file(trusted,args.expected_inputs.name))
record = json.loads(guard.read_file(candidate, 'build-record.json'))
if (record['source_digest']!=expected['source_digest'] or record['source_files']!=expected['source_files']
        or record['profile']!='release' or expected.get('build_completed') is not True
        or record['app_files']!=expected.get('app_files')):
    raise SystemExit('Build does not match the independently expected release inputs')
source = candidate/'source'
files = guard.manifest_files(source, require_index=False, forbidden=rules)
guard.exact_source_tree(source,files)
file_hashes = {name: hashlib.sha256(data).hexdigest() for name, data in files.items()}
if file_hashes != record['source_files'] or hashlib.sha256(json.dumps(file_hashes, sort_keys=True, separators=(',', ':')).encode()).hexdigest() != record['source_digest']:
    raise SystemExit('Frozen source does not match the build record')
# Use the audited packaging implementation belonging to this snapshot.
for name in ['scripts/package.py', 'scripts/privacy_guard.py']:
    if (Path(__file__).resolve().parent.parent/name).read_bytes() != files[name]:
        raise SystemExit('Packaging code differs from the build snapshot')
app = candidate/'Rust Explorer.app'
app_files = guard.bundle_files(app, rules)
if {name: hashlib.sha256(data).hexdigest() for name, data in app_files.items()} != record['app_files']:
    raise SystemExit('App differs from the frozen build output')
if not (record['version']==expected['version']==tomllib.loads(files['Cargo.toml'].decode())['package']['version']==plistlib.loads(app_files['Contents/Info.plist'])['CFBundleShortVersionString']):
    raise SystemExit('Release versions disagree')
if app_files['Contents/Info.plist']!=files['resources/Info.plist'] or app_files['Contents/Resources/ExplorerFolder.icns']!=files['resources/AppIcon.icns']:
    raise SystemExit('Bundle resources differ from reviewed source')
exe=app/'Contents/MacOS/Rust Explorer'
if subprocess.check_output(['/usr/bin/lipo','-archs',str(exe)],text=True).strip()!='arm64':
    raise SystemExit('Unexpected architecture')
load_commands=subprocess.check_output(['/usr/bin/otool','-l',str(exe)],text=True)
if 'cmd LC_RPATH' in load_commands or not re.search(r'\bminos 14\.0\b',load_commands):
    raise SystemExit('Unexpected binary load commands')
guard.certificate_requirement(expected)
subprocess.run(['codesign', '--verify', '--deep', '--strict', '-R', '='+expected['designated_requirement'], str(app)], check=True)
signature = subprocess.run(['codesign', '-d', '-r-', str(app)], capture_output=True, text=True, check=True)
requirement = re.search(r'^designated => (.+)$', signature.stdout+'\n'+signature.stderr, re.M).group(1)
if requirement!=expected['designated_requirement']:
    raise SystemExit('Signing identity differs from trusted inputs')
with tempfile.TemporaryDirectory(prefix='.package-', dir=candidate) as temporary:
    stage = Path(temporary)
    archives = [('Rust-Explorer-macOS-arm64.zip', app_files, 'Rust Explorer.app', ['Contents/MacOS/Rust Explorer']),
                ('Rust-Explorer-source.zip', files, 'Rust-Explorer-source', [name for name in files if name.endswith('.sh')])]
    checksums = {}
    for name, content, prefix, executable in archives:
        path = stage/name
        guard.write_zip(path, content, prefix, executable)
        checksums[name] = guard.audit_zip(path, content, prefix, rules, executable)
    extract = stage/'extracted'
    subprocess.run(['/usr/bin/ditto', '-x', '-k', str(stage/archives[0][0]), str(extract)], check=True)
    restored = extract/'Rust Explorer.app'
    if guard.bundle_files(restored, rules) != app_files:
        raise SystemExit('Restored app bytes differ')
    subprocess.run(['codesign', '--verify', '--deep', '--strict', '-R', '='+requirement, str(restored)], check=True)
    if not (restored/'Contents/MacOS/Rust Explorer').stat().st_mode & 0o111:
        raise SystemExit('Executable mode was not preserved')
    # Construct the complete publish directory, then expose it with one rename.
    ready = stage/'ready'
    ready.mkdir(mode=0o700)
    for name in checksums:
        os.replace(stage/name, ready/name)
    (ready/'SHA256SUMS.txt').write_text(''.join(digest+'  '+name+'\n' for name, digest in checksums.items()))
    report = {'version': record['version'], 'source_digest': record['source_digest'],
              'scoped_privacy_checks_passed': True, 'exact_manifest_and_zip_bytes_verified': True,
              'source_files_verified': len(files), 'extracted_signature_verified': True,
              'binary_sha256': record['app_files']['Contents/MacOS/Rust Explorer'],
              'artifacts': {name: {'sha256': digest, 'bytes': (ready/name).stat().st_size} for name, digest in checksums.items()}}
    (ready/'READY.json').write_text(json.dumps(report, sort_keys=True, indent=2)+'\n')
    os.replace(ready, candidate/'publish')
print(json.dumps(report, sort_keys=True))
