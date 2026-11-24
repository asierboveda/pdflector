import importlib.util
import pathlib
import tempfile
import unittest

SCRIPT = pathlib.Path(__file__).parents[1] / 'master-cloud-sync.py'
spec = importlib.util.spec_from_file_location('master_cloud_sync', SCRIPT)
sync = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sync)

class MirrorTests(unittest.TestCase):
    def test_only_pdfs_are_uploaded_with_tree_preserved(self):
        with tempfile.TemporaryDirectory() as t:
            source, cloud = pathlib.Path(t)/'source', pathlib.Path(t)/'cloud'
            (source/'ML').mkdir(parents=True)
            (source/'.aws').mkdir()
            (source/'_PDFLector').mkdir()
            (source/'ML/lecture.pdf').write_bytes(b'%PDF-1.7\nlecture')
            (source/'ML/private.txt').write_text('not a PDF')
            (source/'.aws/private.pdf').write_bytes(b'not for upload')
            (source/'_PDFLector/notes.pdf').write_bytes(b'output')
            cloud.mkdir()
            sync.mirror(source, cloud, {})
            self.assertEqual((cloud/'ML/lecture.pdf').read_bytes(), b'%PDF-1.7\nlecture')
            self.assertFalse((cloud/'ML/private.txt').exists())
            self.assertFalse((cloud/'.aws').exists())
            self.assertFalse((cloud/'_PDFLector/notes.pdf').exists())

    def test_outputs_return_and_remote_deletion_never_deletes_local_notes(self):
        with tempfile.TemporaryDirectory() as t:
            source, cloud = pathlib.Path(t)/'source', pathlib.Path(t)/'cloud'
            source.mkdir()
            out = cloud/'_PDFLector/Apuntes/ML/book.pdf'
            out.parent.mkdir(parents=True)
            out.write_bytes(b'%PDF notes')
            state = {}
            sync.mirror(source, cloud, state)
            local = source/'_PDFLector/Apuntes/ML/book.pdf'
            self.assertEqual(local.read_bytes(), out.read_bytes())
            out.unlink()
            sync.mirror(source, cloud, state)
            self.assertTrue(local.exists())

    def test_local_output_edit_is_preserved_when_cloud_changes(self):
        with tempfile.TemporaryDirectory() as t:
            source, cloud = pathlib.Path(t)/'source', pathlib.Path(t)/'cloud'
            source.mkdir()
            out = cloud/'_PDFLector/Apuntes/book.pdf'
            out.parent.mkdir(parents=True)
            out.write_bytes(b'%PDF first')
            state = {}
            sync.mirror(source, cloud, state)
            local = source/'_PDFLector/Apuntes/book.pdf'
            local.write_bytes(b'%PDF desktop edit')
            out.write_bytes(b'%PDF tablet edit')
            sync.mirror(source, cloud, state)
            self.assertEqual(local.read_bytes(), out.read_bytes())
            self.assertTrue(any(p.read_bytes() == b'%PDF desktop edit' for p in local.parent.glob('*conflict*')))

    def test_symlinks_are_not_uploaded(self):
        with tempfile.TemporaryDirectory() as t:
            source, cloud = pathlib.Path(t)/'source', pathlib.Path(t)/'cloud'
            source.mkdir(); cloud.mkdir()
            secret = pathlib.Path(t)/'outside.pdf'
            secret.write_bytes(b'outside')
            (source/'escape.pdf').symlink_to(secret)
            sync.mirror(source, cloud, {})
            self.assertFalse((cloud/'escape.pdf').exists())

    def test_upload_collision_does_not_block_independent_downloads_and_retries(self):
        with tempfile.TemporaryDirectory() as t:
            source, cloud = pathlib.Path(t)/'source', pathlib.Path(t)/'cloud'
            source.mkdir()
            (cloud/'book.pdf').mkdir(parents=True)  # directory colliding with PDF path
            (source/'book.pdf').write_bytes(b'%PDF collided')
            (source/'other.pdf').write_bytes(b'%PDF ok')
            note = cloud/'_PDFLector/Apuntes/note.pdf'
            note.parent.mkdir(parents=True)
            note.write_bytes(b'%PDF tablet note')

            state = {}
            uploads, downloads, failures = sync.mirror(source, cloud, state)
            self.assertEqual((uploads, downloads, failures), (1, 1, 1))
            self.assertEqual((cloud/'other.pdf').read_bytes(), b'%PDF ok')
            self.assertEqual((source/'_PDFLector/Apuntes/note.pdf').read_bytes(), b'%PDF tablet note')

            (cloud/'book.pdf').rmdir()
            uploads, downloads, failures = sync.mirror(source, cloud, state)
            self.assertEqual((uploads, downloads, failures), (1, 0, 0))
            self.assertEqual((cloud/'book.pdf').read_bytes(), b'%PDF collided')

if __name__ == '__main__':
    unittest.main()
