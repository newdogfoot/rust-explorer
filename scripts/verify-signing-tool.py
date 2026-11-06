#!/usr/bin/env python3
"""Detect changes to a locally built signer; no signing key is read here."""
from pathlib import Path
import hashlib
import json
import os
import sys

root = Path(__file__).resolve().parent.parent
tools = Path(os.environ.get('RUST_EXPLORER_TOOLCHAIN_ROOT', root/'.tools')).resolve()
folder = tools/'signing'
exe = folder/'bin/rcodesign'
record_path = folder/'verified-tool.json'
lock = root/'scripts/signing-tool.lock'
source_pin = 'f24e9ebdb70a2aee3ca1cea217009fb50776955f0d7678c31d22e48c1524667f'

def digest(p): return hashlib.sha256(p.read_bytes()).hexdigest()

try:
    current = {'executable_sha256': digest(exe), 'lock_sha256': digest(lock), 'source_crate_sha256': source_pin}
    if sys.argv[1:] == ['--record-after-build']:
        archive = next((tools/'cargo/registry/cache').glob('*/apple-codesign-0.29.0.crate'))
        if digest(archive) != source_pin: raise ValueError('Source mismatch')
        record_path.write_text(json.dumps(current, sort_keys=True)+'\n')
        record_path.chmod(0o600)
    else:
        if json.loads(record_path.read_text()) != current: raise ValueError('Tool mismatch')
except (OSError, ValueError, AssertionError, StopIteration):
    raise SystemExit(1)
