#!/usr/bin/env python3
"""Negative tests for the scanner boundary; no network or package execution."""
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import dependency_review as gate


class ReviewTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / 'repo'
        self.repo.mkdir()
        self.git('init', '-q')
        self.git('config', 'user.name', 'Fixture')
        self.git('config', 'user.email', 'fixture@example.invalid')
        (self.repo / 'Cargo.lock').write_text('clean')
        self.base = self.commit()
        self.binary = self.root / 'scanner'
        self.binary.write_text('''#!/usr/bin/env python3
import json,sys,pathlib
args=sys.argv[1:];tree=pathlib.Path(args[-1]);out=pathlib.Path(next(s.split('=',1)[1] for s in args if s.startswith('--output-file=')))
mode=(tree/'Cargo.lock').read_text()
if mode=='missing':sys.exit(1)
vulns=[{'id':'GHSA-fixture'}] if mode=='vulnerable' else []
report={'results':[{'source':{'path':str(tree/'Cargo.lock'),'type':'lockfile'},'packages':[{'package':{'ecosystem':'crates.io','name':'fixture','version':'1'},'vulnerabilities':vulns}]}]}
out.write_text('invalid' if mode=='malformed' else json.dumps(report))
sys.exit(129 if mode=='partial' else 1 if vulns else 0)
''')
        self.binary.chmod(0o700)

    def git(self, *args):
        return subprocess.check_output(['git', '-C', str(self.repo), *args], stderr=subprocess.DEVNULL, text=True).strip()

    def commit(self):
        self.git('add', '.')
        self.git('commit', '-qm', 'fixture')
        return self.git('rev-parse', 'HEAD')

    def compare(self, mode, reports=False):
        (self.repo / 'Cargo.lock').write_text(mode)
        if reports:
            for name in ('old-results.json', 'new-results.json', 'base.json', 'head.json'):
                (self.repo / name).write_text('{"results":[]}')
            (self.repo / 'osv-scanner.toml').write_text('[[PackageOverrides]]\nignore=true\n')
        head = self.commit()
        with patch.object(gate, 'verify_scanner'):
            return gate.compare(self.repo, self.base, head, self.binary, self.root / 'evidence')

    def test_committed_reports_cannot_replace_trusted_baseline(self):
        self.assertEqual(self.compare('vulnerable', reports=True), 1)
        result = json.loads((self.root / 'evidence/comparison.json').read_text())
        self.assertEqual(len(result['introduced']), 1)
        self.assertEqual(result['base_findings'], 0)

    def test_missing_report_cannot_reuse_committed_report(self):
        with self.assertRaises(gate.ScanError):
            self.compare('missing', reports=True)

    def test_api_failure_with_nonempty_partial_report_fails(self):
        with self.assertRaises(gate.ScanError):
            self.compare('partial')

    def test_malformed_report_fails(self):
        with self.assertRaises(ValueError):
            self.compare('malformed')

    def test_unchanged_dependency_findings_pass(self):
        self.assertEqual(self.compare('clean', reports=True), 0)

    def test_untrusted_binary_fails(self):
        with self.assertRaises(gate.ScanError):
            gate.verify_scanner(self.binary)

    def test_evidence_cannot_be_under_checkout(self):
        with self.assertRaises(gate.ScanError):
            gate.compare(self.repo, self.base, self.base, self.binary, self.repo / 'reports')

    def test_stale_external_report_is_rejected(self):
        report = self.root / 'stale.json'
        report.write_text('{"results":[]}')
        with self.assertRaises(gate.ScanError):
            gate.scan(self.binary, self.repo, report, self.root / 'config')

    def test_missing_inventory_is_rejected(self):
        self.binary.write_text('#!/usr/bin/env python3\nimport sys,pathlib\nout=next(s.split("=",1)[1] for s in sys.argv if s.startswith("--output-file="));pathlib.Path(out).write_text(\'{"results":[]}\')\n')
        with self.assertRaises(gate.ScanError):
            self.compare('empty inventory')


if __name__ == '__main__':
    unittest.main()
