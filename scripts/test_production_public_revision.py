import unittest
from check_production_public_revision import validate
class RevisionTests(unittest.TestCase):
    def test_equal_public_revision(self):self.assertEqual(validate(['a'*40]*2,lambda _:True),'a'*40)
    def test_private_missing_or_mixed_revision(self):
        for revisions,reachable in [(['a'*40]*2,False),(['a'*40,'b'*40],True),(['a'*40,''],True),(['a'*8]*2,True)]:
            with self.subTest(revisions=revisions),self.assertRaises(ValueError):validate(revisions,lambda _:reachable)
if __name__=='__main__':unittest.main()
