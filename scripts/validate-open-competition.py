import os

def test_open_competition_v1_readiness():
    path = "docs/OPEN_COMPETITION_V1_FEEDBACK.md"
    assert os.path.exists(path), f"Missing {path}"
    with open(path, "r", encoding="utf-8") as f:
        content = f.read()
    assert "Open Competition V1" in content
    assert "BountySettled" in content
    print("✅ Open Competition V1 validation passed")

if __name__ == "__main__":
    test_open_competition_v1_readiness()
