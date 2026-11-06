#!/usr/bin/env python3
"""Fail-closed publication checks. Diagnostics never echo matched private values."""
from pathlib import Path, PurePosixPath
import argparse
import base64
import hashlib
import io
import json
import os
import re
import stat
import struct
import subprocess
import unicodedata
import zipfile
import zlib
from urllib.parse import unquote_to_bytes

PNG_SIGNATURE = b'\x89PNG\r\n\x1a\n'
PNG_RENDER_CHUNKS = {b'IHDR', b'PLTE', b'IDAT', b'IEND', b'tRNS', b'sRGB', b'gAMA', b'cHRM', b'sBIT'}
ICNS_IMAGE_TYPES = {b'ic04', b'ic05', b'icp4', b'icp5', b'icp6', b'ic07', b'ic08', b'ic09', b'ic10', b'ic11', b'ic12', b'ic13', b'ic14', b'is32', b'il32', b'ih32', b'it32', b's8mk', b'l8mk', b'h8mk', b't8mk'}
# The only reviewed legacy (non-PNG) representations of this app's artwork.
LEGACY_ICON_HASHES = {b'ic04': '6d6199f8c171052ef4f02e01a19b4e95976db2d148d8de950f330878e86ac732', b'ic05': '5d47aaad04238b67f17209e245bbceb57e9d6f20afae67a068cc8fc2f1b88f2c'}
REVIEWED_IMAGES = {
    'resources/AppIcon.png': '97dad6910c42424d6c28f10e6c790ce5cafe22c32b5b171a555edc599638a843',
    'resources/AppIcon.icns': '8f281ea25e89c01ddac0cc7bafbd20db73d19caa54f28a820b74fc5618f516fb',
}
SOURCE_TEXT_SUFFIXES = {'.rs', '.py', '.sh', '.toml', '.lock', '.md', '.txt', '.plist', '.html'}
REVIEWED_FIXTURES = {
    'tests/fixtures/unicode.zip': '6e8edba1e2c138d3c9b9768b7f0b2ac365366b50815093ceb01f3317e693f8fb',
    'tests/fixtures/encrypted.zip': '22d57073bddd39c7712d3ba1bb5d166a414ad7101beb764a171443eb6b71bea4',
}
APP_FILES = {
    'Contents/Info.plist',
    'Contents/MacOS/Rust Explorer',
    'Contents/Resources/ExplorerFolder.icns',
    'Contents/_CodeSignature/CodeResources',
}
PRIVATE_COMPONENTS = {'.git', '.tools', '.test-data', '__MACOSX', '__pycache__', 'node_modules', '.DS_Store', 'HANDOFF.md', '.env'}
PATTERNS = {
    'developer-home-path': re.compile(rb'(?:/Users|/home)/[A-Za-z0-9][A-Za-z0-9_.-]*(?=/|[\\\x00\s])'),
    'windows-user-path': re.compile(rb'[A-Za-z]:[\\/]+Users[\\/]+[A-Za-z0-9][A-Za-z0-9_.-]*', re.I),
    'private-key': re.compile(rb'-----BEGIN (?:RSA |EC |OPENSSH |ENCRYPTED )?PRIVATE KEY-----'),
    'github-credential': re.compile(rb'(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{30,})'),
    'api-credential': re.compile(rb'sk-(?:proj-|svcacct-)?[A-Za-z0-9_-]{32,}'),
    'aws-credential': re.compile(rb'(?:AKIA|ASIA)[A-Z0-9]{16}'),
    'slack-credential': re.compile(rb'xox[baprs]-[A-Za-z0-9-]{20,}'),
}


class Rejected(ValueError):
    pass


def certificate_requirement(record):
    pin = record.get('certificate_sha1', '')
    if not isinstance(pin, str) or not re.fullmatch('[0-9a-f]{40}', pin):
        raise Rejected('Missing exact certificate fingerprint')
    requirement = 'identifier "local.rust-explorer.app" and certificate leaf = H"' + pin + '"'
    if record.get('designated_requirement') != requirement:
        raise Rejected('Certificate requirement does not match the pinned fingerprint')
    return requirement


def fail(path, reason):
    location = hashlib.sha256(str(path).encode()).hexdigest()[:12]
    raise Rejected(f'file-id {location}: {reason}')


def private_rules(path=None):
    # User-specific rules stay outside the public manifest and all output files.
    values = [str(Path.home()), str(Path(__file__).resolve().parent.parent)]
    path = path or os.environ.get('RUST_EXPLORER_PRIVATE_RULES')
    if path:
        document = json.loads(Path(path).read_text())
        values += document['forbidden_values']
    return [value.encode() for value in values if len(value) >= 4]


def audit_bytes(data, name, forbidden=(), depth=0):
    for reason, pattern in PATTERNS.items():
        if pattern.search(data):
            fail(name, reason)
    folded = data.lower()
    for value in forbidden:
        if value.lower() in folded:
            fail(name, 'private-review-rule')
        if base64.b64encode(value) in data or value.decode('utf-8').encode('utf-16-le') in data or value.decode('utf-8').encode('utf-16-be') in data:
            fail(name, 'encoded-private-review-rule')
    if depth >= 2:
        return
    variants = []
    if b'\x00' in data:
        variants.append(data.replace(b'\x00', b''))
        for encoding in ('utf-16-le', 'utf-16-be', 'utf-32-le', 'utf-32-be'):
            try:
                variants.append(data.decode(encoding).encode('utf-8'))
            except UnicodeError:
                pass
    if re.search(rb'\\u[0-9a-fA-F]{4}', data):
        variants.append(re.sub(rb'\\u([0-9a-fA-F]{4})', lambda m: chr(int(m[1], 16)).encode('utf-8', 'surrogatepass'), data))
    if re.search(rb'%[0-9a-fA-F]{2}', data):
        variants.append(unquote_to_bytes(data))
    for token in re.findall(rb'[A-Za-z0-9+/]{24,}={0,2}', data):
        try:
            variants.append(base64.b64decode(token, validate=True))
        except ValueError:
            pass
    for variant in variants:
        if variant != data:
            audit_bytes(variant, name, forbidden, depth+1)


def png_chunks(data, name):
    if not data.startswith(PNG_SIGNATURE):
        fail(name, 'invalid PNG signature')
    offset, chunks = 8, []
    while offset < len(data):
        if len(data) - offset < 12:
            fail(name, 'truncated PNG chunk')
        size = struct.unpack_from('>I', data, offset)[0]
        end = offset + 12 + size
        if end > len(data):
            fail(name, 'invalid PNG chunk length')
        kind, payload = data[offset+4:offset+8], data[offset+8:end-4]
        if zlib.crc32(kind + payload) != struct.unpack_from('>I', data, end-4)[0]:
            fail(name, 'PNG checksum mismatch')
        chunks.append((kind, payload, data[offset:end]))
        offset = end
        if kind == b'IEND':
            if offset != len(data):
                fail(name, 'data after PNG end')
            break
    if not chunks or chunks[0][0] != b'IHDR' or chunks[-1][0] != b'IEND':
        fail(name, 'incomplete PNG')
    return chunks


def icns_elements(data, name):
    if len(data) < 8 or data[:4] != b'icns' or struct.unpack_from('>I', data, 4)[0] != len(data):
        fail(name, 'invalid ICNS container')
    offset, entries = 8, []
    while offset < len(data):
        if len(data) - offset < 8:
            fail(name, 'truncated ICNS entry')
        kind, size = data[offset:offset+4], struct.unpack_from('>I', data, offset+4)[0]
        if size < 8 or offset + size > len(data):
            fail(name, 'invalid ICNS entry length')
        entries.append((kind, data[offset+8:offset+size]))
        offset += size
    if not entries:
        fail(name, 'empty ICNS container')
    return entries


def audit_image(data, name):
    name = name.lower()
    if name.endswith('.png'):
        if any(kind not in PNG_RENDER_CHUNKS for kind, _, _ in png_chunks(data, name)):
            fail(name, 'non-rendering PNG metadata')
    elif name.endswith('.icns'):
        for kind, payload in icns_elements(data, name):
            if kind not in ICNS_IMAGE_TYPES:
                fail(name, 'unexpected ICNS metadata/member')
            if payload.startswith(PNG_SIGNATURE):
                audit_image(payload, name + '.png')
            elif LEGACY_ICON_HASHES.get(kind) != hashlib.sha256(payload).hexdigest():
                fail(name, 'unreviewed legacy/unknown icon encoding')


def sanitize_image(data, name):
    """Remove container metadata without changing compressed pixel/color data."""
    if name.endswith('.png'):
        chunks = png_chunks(data, name)
        result = PNG_SIGNATURE + b''.join(raw for kind, _, raw in chunks if kind in PNG_RENDER_CHUNKS)
        assert [(k, v) for k, v, _ in png_chunks(result, name)] == [(k, v) for k, v, _ in chunks if k in PNG_RENDER_CHUNKS]
    elif name.endswith('.icns'):
        members = []
        for kind, payload in icns_elements(data, name):
            if kind in (b'info', b'TOC '):
                continue
            if kind not in ICNS_IMAGE_TYPES:
                fail(name, 'unknown ICNS entry cannot be sanitized safely')
            if payload.startswith(PNG_SIGNATURE):
                payload = sanitize_image(payload, name + '.png')
            members.append(kind + struct.pack('>I', len(payload)+8) + payload)
        body = b''.join(members)
        result = b'icns' + struct.pack('>I', len(body)+8) + body
    else:
        fail(name, 'unsupported image type')
    audit_image(result, name)
    return result


def safe_relative(name):
    if not name or '\\' in name or '\x00' in name:
        fail('<path>', 'invalid relative path')
    path = PurePosixPath(name)
    if path.is_absolute() or any(part in ('', '.', '..') for part in name.split('/')) or str(path) != name:
        fail('<path>', 'unsafe relative path')
    if name != unicodedata.normalize('NFC', name):
        fail(name, 'noncanonical path encoding')
    if any(part.casefold() in {p.casefold() for p in PRIVATE_COMPONENTS} or part.startswith('._') for part in path.parts):
        fail(name, 'private/internal path')
    if any(part.casefold().startswith('.env.') for part in path.parts) or path.suffix.lower() in ('.key', '.pem', '.p12', '.pfx', '.sqlite', '.db', '.ips'):
        fail(name, 'private file type')
    if name.casefold().startswith(('docs/reviews/', 'docs/validation/', 'dist/')):
        fail(name, 'internal output directory')
    return name


def open_root(root):
    root = Path(root).absolute()
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
    fd = os.open('/', flags)
    try:
        for component in root.parts[1:]:
            next_fd = os.open(component, flags, dir_fd=fd)
            os.close(fd)
            fd = next_fd
        return fd
    except BaseException:
        os.close(fd)
        fail('<root>', 'root/ancestor is not a direct readable directory')


def checked_root(root):
    fd = open_root(root)
    os.close(fd)
    return Path(root).absolute()


def read_file(root, name):
    root = checked_root(root)
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
    descriptors = [open_root(root)]
    try:
        parts = PurePosixPath(name).parts
        for part in parts[:-1]:
            descriptors.append(os.open(part, flags, dir_fd=descriptors[-1]))
        fd = os.open(parts[-1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=descriptors[-1])
        descriptors.append(fd)
        before = os.fstat(fd)
        if not stat.S_ISREG(before.st_mode) or before.st_size > 32*1024*1024:
            fail(name, 'not an allowed-size regular file')
        chunks = []
        while part := os.read(fd, 1024*1024):
            chunks.append(part)
            if sum(map(len, chunks)) > 32*1024*1024:
                fail(name, 'file grew during read')
        after = os.fstat(fd)
        if (before.st_size, before.st_mtime_ns, before.st_ctime_ns) != (after.st_size, after.st_mtime_ns, after.st_ctime_ns):
            fail(name, 'file changed during read')
        return b''.join(chunks)
    finally:
        for fd in reversed(descriptors):
            os.close(fd)


def manifest_files(project, require_index=True, forbidden=()):
    project = checked_root(project)
    manifest = read_file(project, 'release/public-files.txt')
    audit_bytes(manifest, '<manifest>', forbidden)
    names = [line.strip() for line in manifest.decode().splitlines() if line.strip() and not line.startswith('#')]
    normalized = [unicodedata.normalize('NFC', n).casefold() for n in names]
    if len(set(normalized)) != len(names) or 'release/public-files.txt' not in names:
        fail('release/public-files.txt', 'duplicate entries or missing self entry')
    files = {}
    for name in names:
        safe_relative(name)
        audit_bytes(name.encode(), '<source-path>', forbidden)
        data = read_file(project, name)
        audit_bytes(data, name, forbidden)
        audit_image(data, name)
        suffix = PurePosixPath(name).suffix.lower()
        if suffix in ('.png', '.icns') and REVIEWED_IMAGES.get(name) != hashlib.sha256(data).hexdigest():
            fail(name, 'image requires independent visual/metadata review')
        if name in REVIEWED_FIXTURES:
            if hashlib.sha256(data).hexdigest() != REVIEWED_FIXTURES[name]:
                fail(name, 'unreviewed archive fixture bytes')
            with zipfile.ZipFile(io.BytesIO(data)) as archive:
                if archive.comment or len(archive.infolist()) != 1:
                    fail(name, 'unexpected fixture structure')
                for item in archive.infolist():
                    if item.extra or item.comment or item.date_time != (2020, 1, 1, 0, 0, 0):
                        fail(name, 'fixture contains metadata')
                    audit_bytes(item.filename.encode(), '<fixture-path>', forbidden)
                    if not item.flag_bits & 1:
                        audit_bytes(archive.read(item), '<fixture-content>', forbidden)
        elif suffix not in ('.png', '.icns'):
            if suffix not in SOURCE_TEXT_SUFFIXES and name != '.gitignore':
                fail(name, 'unreviewed source file type/archive')
            if b'\x00' in data:
                fail(name, 'binary content in source text')
            try:
                data.decode('utf-8')
            except UnicodeError:
                fail(name, 'source text encoding is not UTF-8')
        if require_index:
            stored = subprocess.run(['git', '-C', str(project), 'show', ':' + name], capture_output=True)
            if stored.returncode or stored.stdout != data:
                fail(name, 'not identical to the reviewed Git index')
        files[name] = data
    return files


def exact_source_tree(root, expected):
    root = checked_root(root)
    wanted_dirs = {str(p) for name in expected for p in PurePosixPath(name).parents if str(p) != '.'}
    actual = set()
    for path in root.rglob('*'):
        name = path.relative_to(root).as_posix()
        if path.is_symlink():
            fail('<snapshot>', 'source tree symlink')
        if path.is_dir():
            if name not in wanted_dirs:
                fail('<snapshot>', 'unexpected source directory')
        elif path.is_file():
            actual.add(name)
        else:
            fail('<snapshot>', 'special source tree entry')
    if actual != set(expected):
        fail('<snapshot>', 'source tree extra/missing file')


def bundle_files(app, forbidden=()):
    app = checked_root(app)
    actual = set()
    directories = {str(parent) for name in APP_FILES for parent in PurePosixPath(name).parents if str(parent) != '.'}
    for path in app.rglob('*'):
        name = path.relative_to(app).as_posix()
        if path.is_symlink():
            fail(name, 'bundle symlink')
        if path.is_file():
            actual.add(name)
        elif path.is_dir() and name not in directories:
            fail('<app>', 'unexpected bundle directory')
        elif not path.is_dir():
            fail(name, 'special bundle file')
    if actual != APP_FILES:
        fail('<app>', 'bundle file manifest mismatch')
    result = {}
    for name in sorted(APP_FILES):
        data = read_file(app, name)
        audit_bytes(data, name, forbidden)
        audit_image(data, name)
        result[name] = data
    return result


def write_zip(destination, files, prefix, executable=()):
    """Content-only ZIP: no filesystem xattrs, resource forks, UID/GID or comments."""
    prefix = safe_relative(prefix)
    paths = {prefix + '/' + safe_relative(name): data for name, data in files.items()}
    directories = {str(parent) + '/' for name in paths for parent in PurePosixPath(name).parents if str(parent) != '.'}
    with zipfile.ZipFile(destination, 'w', zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for name in sorted(directories) + sorted(paths):
            directory = name.endswith('/')
            info = zipfile.ZipInfo(name, date_time=(2020, 1, 1, 0, 0, 0))
            info.create_system = 3
            mode = (stat.S_IFDIR | 0o755) if directory else (stat.S_IFREG | (0o755 if name[len(prefix)+1:] in executable else 0o644))
            info.external_attr = (mode << 16) | (0x10 if directory else 0)
            info.compress_type = zipfile.ZIP_STORED if directory else zipfile.ZIP_DEFLATED
            archive.writestr(info, b'' if directory else paths[name])


def audit_zip(path, expected, prefix, forbidden=(), executable=()):
    seen, files = set(), {}
    expected_dirs = {str(parent) + '/' for name in expected for parent in PurePosixPath(prefix + '/' + name).parents if str(parent) != '.'}
    with zipfile.ZipFile(path) as archive:
        if archive.comment:
            fail('<zip>', 'archive comment')
        for item in archive.infolist():
            name = item.filename
            canonical = unicodedata.normalize('NFC', name).casefold()
            if canonical in seen:
                fail('<zip>', 'duplicate archive member')
            seen.add(canonical)
            safe_relative(name.rstrip('/'))
            if item.extra or item.comment or item.flag_bits & 1:
                fail(name, 'archive metadata or encryption')
            if item.is_dir():
                if name not in expected_dirs or item.file_size or item.external_attr >> 16 != stat.S_IFDIR | 0o755:
                    fail(name, 'unexpected archive directory')
                continue
            if not name.startswith(prefix + '/'):
                fail(name, 'archive path outside prefix')
            relative = name[len(prefix)+1:]
            if relative not in expected or item.file_size != len(expected[relative]):
                fail(name, 'unexpected archive file or size')
            mode = item.external_attr >> 16
            wanted = stat.S_IFREG | (0o755 if relative in executable else 0o644)
            if mode != wanted:
                fail(name, 'unexpected archive permissions/type')
            data = archive.read(item)
            if data != expected[relative]:
                fail(name, 'archive content mismatch')
            audit_bytes(data, name, forbidden)
            audit_image(data, name)
            files[relative] = data
    if set(files) != set(expected):
        fail('<zip>', 'archive missing files')
    # Reconstruct the canonical archive to validate local headers, all extra
    # fields, offsets, timestamp fields and unparsed prefix/trailer bytes too.
    canonical = io.BytesIO()
    write_zip(canonical, expected, prefix, executable)
    if Path(path).read_bytes() != canonical.getvalue():
        fail('<zip>', 'noncanonical archive bytes or hidden metadata')
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def export_source(project, destination, forbidden=()):
    if destination.exists():
        fail('<export>', 'destination must be a new directory')
    files = manifest_files(project, forbidden=forbidden)
    checked_root(destination.parent)
    destination.mkdir(parents=True, mode=0o700)
    for name, data in files.items():
        path = destination / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        path.chmod(0o755 if name.endswith('.sh') else 0o644)
    actual = {p.relative_to(destination).as_posix() for p in destination.rglob('*') if p.is_file()}
    if actual != set(files):
        fail('<export>', 'source export manifest mismatch')
    exact_source_tree(destination, files)
    return {name: hashlib.sha256(data).hexdigest() for name, data in files.items()}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['source', 'app', 'export', 'sanitize-image'])
    parser.add_argument('path', type=Path)
    parser.add_argument('--destination', type=Path)
    parser.add_argument('--private-rules', type=Path)
    args = parser.parse_args()
    try:
        rules = private_rules(args.private_rules)
        if args.action == 'source':
            result = {'source_files_checked': len(manifest_files(args.path, forbidden=rules))}
        elif args.action == 'app':
            result = {'app_files_checked': len(bundle_files(args.path, forbidden=rules))}
        elif args.action == 'export':
            if not args.destination:
                parser.error('--destination is required')
            result = {'files': export_source(args.path, args.destination, rules)}
        else:
            if not args.destination or args.destination == args.path:
                parser.error('a separate --destination is required')
            clean = sanitize_image(args.path.read_bytes(), args.path.name)
            audit_bytes(clean, args.path.name, rules)
            args.destination.write_bytes(clean)
            result = {'render_data_preserved': True}
        print(json.dumps(result, sort_keys=True))
    except Rejected as error:
        raise SystemExit('Publication check failed: ' + str(error))
    except (OSError, ValueError):
        raise SystemExit('Publication check failed: invalid input or I/O failure')
