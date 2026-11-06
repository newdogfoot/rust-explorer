#!/usr/bin/env python3
"""Adversarial publication-gate checks; only synthetic data and private temp dirs."""
from pathlib import Path
import base64
import importlib.util
import io
import json
import os
import stat
import struct
import subprocess
import tempfile
import unittest
import zipfile
import zlib
import privacy_guard as guard


def png_chunk(kind, data):
    return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind+data))


def tiny_png(extra=b''):
    return (guard.PNG_SIGNATURE + png_chunk(b'IHDR', struct.pack('>IIBBBBB', 1, 1, 8, 6, 0, 0, 0)) + extra
            + png_chunk(b'IDAT', zlib.compress(b'\0\xff\0\0\xff')) + png_chunk(b'IEND', b''))


class PrivacyGateTests(unittest.TestCase):
    def test_certificate_requires_exact_pin_and_identifier(self):
        pin = 'a'*40
        requirement = 'identifier "local.rust-explorer.app" and certificate leaf = H"' + pin + '"'
        record = {'certificate_sha1':pin, 'designated_requirement':requirement}
        self.assertEqual(guard.certificate_requirement(record), requirement)
        for invalid in [{}, dict(record, certificate_sha1='b'*40),
                        dict(record, designated_requirement='certificate leaf[subject.CN] = "fixture"'),
                        dict(record, designated_requirement=requirement+' or true'),
                        dict(record, certificate_sha1=42)]:
            with self.assertRaises(guard.Rejected): guard.certificate_requirement(invalid)

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='release-gate-')
        self.root = Path(self.temp.name).resolve()

    def tearDown(self):
        self.temp.cleanup()

    def test_plain_and_encoded_private_values(self):
        value = b'/' + b'Users/' + b'example-person/private-note'
        variants = [value, value.decode().encode('utf-16-le'), value.decode().encode('utf-16-be'),
                    value.decode().encode('utf-32-le'), base64.b64encode(value), base64.b64encode(base64.b64encode(value)),
                    value.replace(b'/', b'\\' + b'u002f'), value.replace(b'/', b'%2F')]
        for data in variants:
            with self.subTest(encoding=data[:3]), self.assertRaises(guard.Rejected):
                guard.audit_bytes(data, 'synthetic')

    def test_dummy_credentials_and_private_rules(self):
        for data in [b'ghp_' + b'A'*36, b'sk-proj-' + b'b'*50,
                     b'-----BEGIN ' + b'OPENSSH PRIVATE KEY-----', b'AKIA' + b'Q'*16]:
            with self.assertRaises(guard.Rejected):
                guard.audit_bytes(data, 'synthetic')
        with self.assertRaises(guard.Rejected) as error:
            guard.audit_bytes(b'do-not-share-this', 'private-personal-filename', [b'do-not-share-this'])
        self.assertNotIn('private-personal-filename', str(error.exception))
        self.assertNotIn('do-not-share-this', str(error.exception))

    def test_private_rules_environment_is_used(self):
        rules = self.root/'rules.json'
        rules.write_text(json.dumps({'forbidden_values': ['fixture-only-private-value']}))
        old = os.environ.get('RUST_EXPLORER_PRIVATE_RULES')
        try:
            os.environ['RUST_EXPLORER_PRIVATE_RULES'] = str(rules)
            self.assertIn(b'fixture-only-private-value', guard.private_rules())
            os.environ['RUST_EXPLORER_PRIVATE_RULES'] = str(self.root/'missing')
            with self.assertRaises(OSError): guard.private_rules()
        finally:
            if old is None: os.environ.pop('RUST_EXPLORER_PRIVATE_RULES', None)
            else: os.environ['RUST_EXPLORER_PRIVATE_RULES'] = old

    def source(self, extra=None):
        project = self.root/'project'
        project.mkdir()
        (project/'release').mkdir()
        content = {'README.md': b'Public fixture\n'}
        content.update(extra or {})
        for name, data in content.items():
            path = project/name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        (project/'release/public-files.txt').write_text('\n'.join(sorted([*content, 'release/public-files.txt']))+'\n')
        subprocess.run(['git', 'init', '-q', str(project)], check=True)
        subprocess.run(['git', '-C', str(project), 'add', '.'], check=True)
        return project

    def test_exact_index_export_ignores_untracked_private_files(self):
        project = self.source()
        (project/'.env').write_text('fake private state')
        (project/'src').mkdir()
        (project/'src/forgotten.txt').write_text('fake private state')
        output = self.root/'export'
        report = guard.export_source(project, output)
        self.assertEqual(set(report), {'README.md', 'release/public-files.txt'})
        self.assertEqual(set(report), {p.relative_to(output).as_posix() for p in output.rglob('*') if p.is_file()})
        self.assertFalse((output/'.git').exists())
        (project/'README.md').write_text('Changed after review')
        with self.assertRaises(guard.Rejected): guard.manifest_files(project)

    def test_frozen_snapshot_rejects_extra_inputs(self):
        project=self.source()
        output=self.root/'export'
        files=guard.export_source(project,output)
        guard.exact_source_tree(output,files)
        (output/'.cargo').mkdir()
        with self.assertRaises(guard.Rejected):guard.exact_source_tree(output,files)
        (output/'.cargo').rmdir()
        (output/'build.rs').write_text('unexpected build script')
        with self.assertRaises(guard.Rejected):guard.exact_source_tree(output,files)

    def test_private_path_and_case_policy(self):
        for name in ['.ENV', 'docs/VALIDATION/private.txt', 'HANDOFF.md', 'notes.key', '../x', 'x/../a', '/absolute', 'a\\b', '._name']:
            with self.subTest(name=name), self.assertRaises(guard.Rejected): guard.safe_relative(name)

    def test_binary_and_nested_archive_not_source_text(self):
        data = io.BytesIO()
        with zipfile.ZipFile(data, 'w', zipfile.ZIP_DEFLATED) as archive:
            archive.writestr('note', b'/' + b'Users/' + b'synthetic-user')
        project = self.source({'fixture.txt': data.getvalue()})
        with self.assertRaises(guard.Rejected): guard.manifest_files(project)

    def test_symlinked_roots_and_components(self):
        project = self.source()
        alias = self.root/'alias'
        alias.symlink_to(project, target_is_directory=True)
        with self.assertRaises(guard.Rejected): guard.manifest_files(alias)
        (project/'README.md').unlink()
        (project/'README.md').symlink_to(project/'release/public-files.txt')
        with self.assertRaises((guard.Rejected, OSError)): guard.manifest_files(project)

    def test_png_metadata_and_render_bytes(self):
        original = tiny_png(png_chunk(b'iCCP', b'display-specific-profile') + png_chunk(b'eXIf', b'fixture-metadata'))
        with self.assertRaises(guard.Rejected): guard.audit_image(original, 'icon.PNG')
        clean = guard.sanitize_image(original, 'icon.png')
        self.assertEqual(clean, tiny_png())
        guard.audit_image(clean, 'icon.PNG')
        with self.assertRaises(guard.Rejected): guard.audit_image(clean+b'trailer', 'icon.png')

    def test_icns_rejects_unreviewed_payload(self):
        payload = b'arbitrary unknown image encoding'
        body = b'ic10'+struct.pack('>I', len(payload)+8)+payload
        icon = b'icns'+struct.pack('>I', len(body)+8)+body
        with self.assertRaises(guard.Rejected): guard.audit_image(icon, 'icon.icns')
        guard.audit_image((Path(__file__).resolve().parent.parent/'resources/AppIcon.icns').read_bytes(), 'icon.icns')

    def archive(self):
        path = self.root/'artifact.zip'
        expected = {'folder/file.txt': b'public content'}
        guard.write_zip(path, expected, 'Product')
        guard.audit_zip(path, expected, 'Product')
        return path, expected

    def test_zip_reproducible_and_exact(self):
        path, expected = self.archive()
        other = self.root/'other.zip'
        guard.write_zip(other, expected, 'Product')
        self.assertEqual(path.read_bytes(), other.read_bytes())
        with self.assertRaises(guard.Rejected): guard.audit_zip(path, {'folder/file.txt': b'changed source'}, 'Product')

    def test_zip_rejects_unparsed_trailer_and_prefix(self):
        path, expected = self.archive()
        clean = path.read_bytes()
        for data in [clean+b'unparsed private metadata', b'unparsed private prefix'+clean]:
            path.write_bytes(data)
            with self.assertRaises(guard.Rejected): guard.audit_zip(path, expected, 'Product')

    def test_zip_rejects_metadata_and_directory_symlink_type(self):
        path, expected = self.archive()
        with zipfile.ZipFile(path) as original:
            entries = [(item, original.read(item)) for item in original.infolist()]
        for variant in ['directory_type', 'extra', 'comment', 'timestamp', 'duplicate']:
            with zipfile.ZipFile(path, 'w') as archive:
                for i, (saved, data) in enumerate(entries):
                    item = zipfile.ZipInfo(saved.filename, saved.date_time)
                    item.create_system=3;item.external_attr=saved.external_attr;item.compress_type=saved.compress_type
                    if i==0:
                        if variant=='directory_type':item.external_attr=(stat.S_IFLNK|0o777)<<16
                        elif variant=='extra':item.extra=b'\xfe\xca\x04\x00test'
                        elif variant=='comment':item.comment=b'private metadata'
                        elif variant=='timestamp':item.date_time=(2026,2,3,4,5,6)
                    archive.writestr(item,data)
                if variant=='duplicate':archive.writestr('Product/folder/file.txt', b'public content')
            with self.subTest(variant=variant), self.assertRaises(guard.Rejected): guard.audit_zip(path, expected, 'Product')

    def test_bundle_rejects_unlisted_files_empty_dirs_and_links(self):
        app=self.root/'Example.app'
        for name in guard.APP_FILES:
            p=app/name;p.parent.mkdir(parents=True,exist_ok=True)
            p.write_bytes((Path(__file__).resolve().parent.parent/'resources/AppIcon.icns').read_bytes() if name.endswith('.icns') else b'synthetic bundle content')
        guard.bundle_files(app)
        extra=app/'unnecessary';extra.mkdir()
        with self.assertRaises(guard.Rejected):guard.bundle_files(app)
        extra.rmdir();extra.write_text('private data')
        with self.assertRaises(guard.Rejected):guard.bundle_files(app)
        extra.unlink();alias=self.root/'Alias.app';alias.symlink_to(app,target_is_directory=True)
        with self.assertRaises(guard.Rejected):guard.bundle_files(alias)


if __name__ == '__main__':
    unittest.main(verbosity=2)
