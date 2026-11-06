#!/usr/bin/env python3
"""Copy the registry-verified signer sources, apply the reviewed dependency lock."""
from pathlib import Path
import hashlib
import shutil
import tarfile

root = Path(__file__).resolve().parent.parent
archive = next((root/'.tools/cargo/registry/cache').glob('*/apple-codesign-0.29.0.crate'))
if hashlib.sha256(archive.read_bytes()).hexdigest() != 'f24e9ebdb70a2aee3ca1cea217009fb50776955f0d7678c31d22e48c1524667f':
    raise RuntimeError('Unexpected signing-tool source archive')
target = root/'.tools/signing-source'
if target.exists():
    shutil.rmtree(target)
target.mkdir()
with tarfile.open(archive) as package:
    for member in package.getmembers():
        parts = Path(member.name).parts
        if not parts or parts[0] != 'apple-codesign-0.29.0' or '..' in parts:
            raise RuntimeError('Unsafe signing-tool source path')
        destination = target.joinpath(*parts[1:])
        if member.isdir():
            destination.mkdir(parents=True, exist_ok=True)
        else:
            if not member.isfile():
                raise RuntimeError('Unexpected signing-tool source entry')
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(package.extractfile(member).read())
shutil.copy2(root/'scripts/signing-tool.lock', target/'Cargo.lock')
print('Prepared verified apple-codesign 0.29.0 sources with the pinned updated lockfile.')
