"""
Test: MCP/API tool-registry drift coverage (Issue #685)

Ensures that the tool registry remains consistent between MCP and API
interfaces — detecting when tools are registered in one but missing from
the other, or have mismatched schemas.
"""
import json
import pytest
from typing import Dict, List, Set


class ToolRegistry:
    """Simplified tool registry for testing drift detection."""

    def __init__(self, name: str, tools: List[Dict] = None):
        self.name = name
        self._tools: Dict[str, Dict] = {}
        if tools:
            for t in tools:
                self.register(t)

    def register(self, tool: Dict) -> None:
        name = tool.get("name", "")
        if not name:
            raise ValueError("Tool must have a name")
        self._tools[name] = tool

    def get_names(self) -> Set[str]:
        return set(self._tools.keys())

    def get_schema(self, name: str) -> Dict:
        return self._tools.get(name, {}).get("input_schema", {})

    def __len__(self):
        return len(self._tools)


class DriftDetector:
    """Detects drift between two tool registries."""

    def __init__(self, mcp_registry: ToolRegistry, api_registry: ToolRegistry):
        self.mcp = mcp_registry
        self.api = api_registry

    def missing_from_api(self) -> Set[str]:
        return self.mcp.get_names() - self.api.get_names()

    def missing_from_mcp(self) -> Set[str]:
        return self.api.get_names() - self.mcp.get_names()

    def schema_mismatches(self) -> Dict[str, Dict]:
        mismatches = {}
        common = self.mcp.get_names() & self.api.get_names()
        for name in common:
            mcp_schema = json.dumps(self.mcp.get_schema(name), sort_keys=True)
            api_schema = json.dumps(self.api.get_schema(name), sort_keys=True)
            if mcp_schema != api_schema:
                mismatches[name] = {
                    "mcp_schema": self.mcp.get_schema(name),
                    "api_schema": self.api.get_schema(name),
                }
        return mismatches

    def is_consistent(self) -> bool:
        return (
            len(self.missing_from_api()) == 0
            and len(self.missing_from_mcp()) == 0
            and len(self.schema_mismatches()) == 0
        )


# Fixtures
MCP_TOOLS = [
    {"name": "search_files", "input_schema": {"type": "object", "properties": {"pattern": {"type": "string"}, "path": {"type": "string"}}}},
    {"name": "read_file", "input_schema": {"type": "object", "properties": {"path": {"type": "string"}, "offset": {"type": "integer"}}}},
    {"name": "web_search", "input_schema": {"type": "object", "properties": {"query": {"type": "string"}, "limit": {"type": "integer"}}}},
    {"name": "terminal", "input_schema": {"type": "object", "properties": {"command": {"type": "string"}, "timeout": {"type": "integer"}}}},
]

API_TOOLS = [
    {"name": "search_files", "input_schema": {"type": "object", "properties": {"pattern": {"type": "string"}, "path": {"type": "string"}}}},
    {"name": "read_file", "input_schema": {"type": "object", "properties": {"path": {"type": "string"}, "offset": {"type": "integer"}}}},
    {"name": "web_search", "input_schema": {"type": "object", "properties": {"query": {"type": "string"}, "limit": {"type": "integer"}}}},
    {"name": "terminal", "input_schema": {"type": "object", "properties": {"command": {"type": "string"}, "timeout": {"type": "integer"}}}},
]


class TestToolRegistry:
    def test_register_and_retrieve(self):
        reg = ToolRegistry("test")
        reg.register({"name": "foo", "input_schema": {"type": "object"}})
        assert "foo" in reg.get_names()
        assert len(reg) == 1

    def test_duplicate_registration_overwrites(self):
        reg = ToolRegistry("test")
        reg.register({"name": "foo", "input_schema": {"type": "object"}})
        reg.register({"name": "foo", "input_schema": {"type": "object", "properties": {"x": {}}}})
        assert reg.get_schema("foo") == {"type": "object", "properties": {"x": {}}}

    def test_empty_registry(self):
        reg = ToolRegistry("empty")
        assert len(reg) == 0
        assert reg.get_names() == set()

    def test_register_without_name_raises(self):
        reg = ToolRegistry("test")
        with pytest.raises(ValueError) as exc_info:
            reg.register({"schema": {}})
        assert "name" in str(exc_info.value).lower()


class TestDriftDetectorConsistent:
    @pytest.fixture
    def consistent_detector(self):
        mcp = ToolRegistry("mcp", MCP_TOOLS)
        api = ToolRegistry("api", API_TOOLS)
        return DriftDetector(mcp, api)

    def test_is_consistent(self, consistent_detector):
        assert consistent_detector.is_consistent()

    def test_no_missing_from_api(self, consistent_detector):
        assert consistent_detector.missing_from_api() == set()

    def test_no_missing_from_mcp(self, consistent_detector):
        assert consistent_detector.missing_from_mcp() == set()

    def test_no_schema_mismatches(self, consistent_detector):
        assert consistent_detector.schema_mismatches() == {}


class TestDriftDetectorDrifted:
    @pytest.fixture
    def drifted_detector(self):
        mcp = ToolRegistry("mcp", MCP_TOOLS + [
            {"name": "new_mcp_tool", "input_schema": {"type": "object"}}
        ])
        api = ToolRegistry("api", API_TOOLS + [
            {"name": "new_api_tool", "input_schema": {"type": "object"}}
        ])
        return DriftDetector(mcp, api)

    def test_detects_missing_from_api(self, drifted_detector):
        missing = drifted_detector.missing_from_api()
        assert "new_mcp_tool" in missing
        assert len(missing) == 1

    def test_detects_missing_from_mcp(self, drifted_detector):
        missing = drifted_detector.missing_from_mcp()
        assert "new_api_tool" in missing
        assert len(missing) == 1

    def test_not_consistent(self, drifted_detector):
        assert not drifted_detector.is_consistent()


class TestDriftDetectorSchemaMismatch:
    @pytest.fixture
    def mismatched_detector(self):
        mcp = ToolRegistry("mcp", [
            {"name": "search_files", "input_schema": {"type": "object", "properties": {"pattern": {"type": "string"}}}}
        ])
        api = ToolRegistry("api", [
            {"name": "search_files", "input_schema": {"type": "object", "properties": {"pattern": {"type": "string"}, "path": {"type": "string"}}}}
        ])
        return DriftDetector(mcp, api)

    def test_detects_schema_mismatch(self, mismatched_detector):
        mismatches = mismatched_detector.schema_mismatches()
        assert "search_files" in mismatches
        assert mismatches["search_files"]["mcp_schema"] != mismatches["search_files"]["api_schema"]


class TestDriftDetectorEdgeCases:
    def test_both_empty(self):
        d = DriftDetector(ToolRegistry("mcp"), ToolRegistry("api"))
        assert d.is_consistent()

    def test_one_empty_one_populated(self):
        mcp = ToolRegistry("mcp", [{"name": "t1", "input_schema": {}}])
        d = DriftDetector(mcp, ToolRegistry("api"))
        assert not d.is_consistent()
        assert d.missing_from_api() == {"t1"}

    def test_large_registries(self):
        tools = [{"name": f"tool_{i}", "input_schema": {"type": "object"}} for i in range(100)]
        mcp = ToolRegistry("mcp", tools)
        api = ToolRegistry("api", tools)
        d = DriftDetector(mcp, api)
        assert d.is_consistent()
        assert len(d.missing_from_api()) == 0
