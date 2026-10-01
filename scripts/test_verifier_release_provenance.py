from __future__ import annotations
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

from scripts import verifier_release_provenance as guard

ROOT = Path(__file__).resolve().parents[1]


def identity(value):
    body = {k: v for k, v in value.items() if k != 'release_id'}
    value['release_id'] = 'sha256:' + hashlib.sha256(json.dumps(body, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
    return value


class ProvenanceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.run_git('init', '-b', 'main')
        (self.root / 'worker-source').write_text('reviewed source')
        self.source = self.commit('source')
        self.value = identity({'schema': 'agent-bounties/verifier-release-v1',
            'source_revision': self.source, 'network': 'base-mainnet',
            **{k: 'sha256:' + 'a' * 64 for k in ['worker_build_digest', 'worker_binary_digest', 'signing_runtime_digest', 'profile_registry_digest']},
            'signers': ['0x' + '1' * 40, '0x' + '2' * 40]})
        self.approve(self.value)
        self.trusted = self.commit('protected approval')
        self.run_git('update-ref', 'refs/remotes/origin/main', self.trusted)

    def run_git(self, *args):
        return subprocess.check_output(['git', '-C', str(self.root), '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', '-c', 'core.hooksPath=/dev/null', *args], text=True, stderr=subprocess.DEVNULL).strip()

    def commit(self, message):
        self.run_git('add', '.')
        self.run_git('commit', '-m', message)
        return self.run_git('rev-parse', 'HEAD')

    def approve(self, value):
        path = self.root / '.github/verifier-releases' / (value['release_id'][7:] + '.json')
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value))
        return path

    def check(self, value=None, expected=''):
        return guard.authorize(self.root, self.trusted, json.dumps(value or self.value), expected)

    def test_exact_approved_ancestor_is_accepted_without_executing_source(self):
        self.assertEqual(self.check(), self.value)

    def test_self_hashed_unmerged_source_rejected_even_with_approval_record(self):
        self.run_git('checkout', '-b', 'unreviewed')
        marker = self.root / 'executed'
        (self.root / 'verifier_release.py').write_text(f'from pathlib import Path; Path({str(marker)!r}).touch()')
        unmerged = self.commit('unreviewed helper')
        changed = identity(dict(self.value, source_revision=unmerged))
        self.run_git('checkout', 'main')
        self.approve(changed)
        self.trusted = self.commit('negative control record cannot authorize a branch')
        self.run_git('update-ref', 'refs/remotes/origin/main', self.trusted)
        with self.assertRaisesRegex(ValueError, 'approval record'):
            self.check(changed)
        self.assertFalse(marker.exists())

    def test_rehashed_binary_substitution_is_not_protected_approval(self):
        changed = identity(dict(self.value, worker_binary_digest='sha256:' + 'b' * 64))
        guard.parse_release(json.dumps(changed))  # The old self-hash boundary accepts this.
        with self.assertRaisesRegex(ValueError, 'approval record'):
            self.check(changed)

    def test_uncommitted_approval_does_not_count(self):
        changed = identity(dict(self.value, signing_runtime_digest='sha256:' + 'c' * 64))
        self.approve(changed)
        with self.assertRaisesRegex(ValueError, 'approval record'):
            self.check(changed)

    def test_squash_source_requires_explicit_protected_binding_and_equal_tree(self):
        # Make an unmerged source commit with the exact reviewed main tree.
        tree = self.run_git('rev-parse', self.source + '^{tree}')
        branch_source = self.run_git('commit-tree', tree, '-m', 'pre-squash release')
        changed = identity(dict(self.value, source_revision=branch_source))
        record = self.approve(changed)
        source_record = record.with_suffix('.source.json')
        source_record.write_text(json.dumps({'schema': 'agent-bounties/protected-verifier-source-v1',
            'release_id': changed['release_id'], 'protected_source_revision': self.source}))
        self.trusted = self.commit('explicit squash-equivalent approval')
        self.run_git('update-ref', 'refs/remotes/origin/main', self.trusted)
        self.assertEqual(self.check(changed), changed)
        # The same protected binding must reject an unmerged altered helper tree.
        (self.root / 'verifier_release.py').write_text('raise RuntimeError("must not execute")')
        other = self.commit('unmerged altered helper')
        self.run_git('reset', '--hard', self.trusted)
        altered = identity(dict(self.value, source_revision=other))
        bad_record = self.approve(altered)
        bad_record.with_suffix('.source.json').write_text(json.dumps({'schema': 'agent-bounties/protected-verifier-source-v1',
            'release_id': altered['release_id'], 'protected_source_revision': self.source}))
        self.trusted = self.commit('negative control unequal tree binding')
        self.run_git('update-ref', 'refs/remotes/origin/main', self.trusted)
        with self.assertRaisesRegex(ValueError, 'tree differs'):
            self.check(altered)

    def test_unprotected_workflow_checkout_is_rejected(self):
        (self.root / 'other').write_text('unreviewed workflow')
        self.trusted = self.commit('unprotected workflow')
        with self.assertRaises(subprocess.CalledProcessError):
            self.check()

    def test_wrong_checkout_and_cross_job_revision_change_fail(self):
        with self.assertRaisesRegex(ValueError, 'between authorization'):
            self.check(expected='f' * 40)
        with self.assertRaisesRegex(ValueError, 'protected workflow checkout'):
            guard.authorize(self.root, self.source, json.dumps(self.value))

    def test_duplicate_fields_and_symlink_approval_fail(self):
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            guard.parse_release('{"schema":1,"schema":2}')
        path = self.approve(self.value)
        path.unlink()
        path.symlink_to('../../../worker-source')
        self.trusted = self.commit('symlink negative control')
        self.run_git('update-ref', 'refs/remotes/origin/main', self.trusted)
        with self.assertRaisesRegex(ValueError, 'regular approval'):
            self.check()

    def test_all_four_execution_jobs_gate_before_release_checkout(self):
        cases = [('regression-verifier-runner.yml', 'run-no-secrets'),
                 ('regression-verifier-signer.yml', 'authorize-run'),
                 ('regression-verifier-signer.yml', 'relay'),
                 ('regression-verifier-signing-reusable.yml', 'sign')]
        for workflow, job in cases:
            text = (ROOT / '.github/workflows' / workflow).read_text()
            block = text.split('\n  ' + job + ':\n', 1)[1]
            trusted = block.index('ref: ${{ github.workflow_sha }}')
            authorize = block.index('run: python3 -I scripts/verifier_release_provenance.py')
            candidate = block.index('ref: ${{ steps.provenance.outputs.revision }}')
            self.assertLess(trusted, authorize, job)
            self.assertLess(authorize, candidate, job)
            self.assertIn('fetch-depth: 0', block[trusted:authorize])
            self.assertIn('persist-credentials: false', block[trusted:authorize])
            if job in ('relay', 'sign'):
                self.assertIn('EXPECTED_RELEASE_REVISION: ${{', block[trusted:authorize])
            if 'REGRESSION_VERIFIER_PRIVATE_KEY:' in block:
                self.assertLess(candidate, block.index('REGRESSION_VERIFIER_PRIVATE_KEY:'))
            if 'BASE_KEEPER_PRIVATE_KEY:' in block:
                self.assertLess(candidate, block.index('BASE_KEEPER_PRIVATE_KEY:'))


if __name__ == '__main__':
    unittest.main()
