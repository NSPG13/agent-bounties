import subprocess
from pathlib import Path
import tempfile
import unittest
import check_public_runtime_parity as guard

class PublicRuntimeParityTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.addCleanup(self.tmp.cleanup)
        self.roots=[Path(self.tmp.name)/n for n in ('runtime','public')]
        for root in self.roots:
            subprocess.run(['git','init','-q',str(root)],check=True)
            for name,text in {'Cargo.toml':'[workspace]\n','crates/api/src/main.rs':'const INPUT: &str = include_str!("../../../fixtures/input.json");','fixtures/input.json':'{}'}.items():
                p=root/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_text(text)
            subprocess.run(['git','-C',str(root),'add','.'],check=True)
    def test_equal_source_is_bound(self):
        self.assertEqual(guard.compare(*self.roots)['file_count'],3)
    def test_changed_security_source_rejected(self):
        (self.roots[0]/'crates/api/src/main.rs').write_text('fn unauthenticated() {}')
        with self.assertRaises(ValueError):guard.compare(*self.roots)
    def test_changed_embedded_input_rejected(self):
        (self.roots[1]/'fixtures/input.json').write_text('{"changed":true}')
        with self.assertRaises(ValueError):guard.compare(*self.roots)
    def test_unpublished_tool_source_rejected(self):
        p=self.roots[0]/'crates/api/src/unpublished.rs';p.write_text('fn tool() {}')
        subprocess.run(['git','-C',str(self.roots[0]),'add','.'],check=True)
        with self.assertRaises(ValueError):guard.compare(*self.roots)
    def test_external_included_rust_source_is_bound(self):
        for root in self.roots:
            (root/'crates/api/src/main.rs').write_text('include!("../../../fixtures/body.rs");')
            (root/'fixtures/body.rs').write_text('fn main() {}')
        self.assertEqual(guard.compare(*self.roots)['file_count'],3)
        (self.roots[0]/'fixtures/body.rs').write_text('fn changed() {}')
        with self.assertRaises(ValueError):guard.compare(*self.roots)
    def test_environment_selected_source_is_rejected(self):
        (self.roots[0]/'crates/api/src/main.rs').write_text('include!(env!("PRIVATE_SOURCE"));')
        with self.assertRaises(ValueError):guard.compare(*self.roots)
    def test_symlink_input_rejected(self):
        p=self.roots[0]/'fixtures/input.json';p.unlink();p.symlink_to(self.roots[1]/'fixtures/input.json')
        with self.assertRaises((ValueError,RuntimeError)):guard.compare(*self.roots)

if __name__=='__main__':unittest.main()
