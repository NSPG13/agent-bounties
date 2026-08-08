#!/usr/bin/env python3
"""Validate A2A Agent Card against the official JSON Schema."""
import json, sys, os

AGENT_CARD_PATHS = [
    "site/.well-known/agent-card.json",
    ".well-known/agent-card.json",
    "fixtures/a2a-agent-card.json",
]

REQUIRED_FIELDS = ["name", "description", "url", "provider", "version", "capabilities"]
RECOMMENDED_FIELDS = ["documentationUrl", "iconUrl", "defaultInputModes", "defaultOutputModes"]
VALID_CAPABILITIES = ["streaming", "pushNotifications", "stateTransitionHistory"]

def validate_card(path):
    if not os.path.exists(path):
        return False, f"File not found: {path}"
    try:
        with open(path) as f:
            card = json.load(f)
    except json.JSONDecodeError as e:
        return False, f"Invalid JSON: {e}"
    
    errors = []
    for field in REQUIRED_FIELDS:
        if field not in card:
            errors.append(f"Missing required field: {field}")
    
    if "version" in card and not isinstance(card["version"], str):
        errors.append(f"version must be a string, got {type(card['version']).__name__}")
    
    if "capabilities" in card:
        caps = card["capabilities"]
        if not isinstance(caps, dict):
            errors.append("capabilities must be an object")
        else:
            for cap in caps:
                if cap not in VALID_CAPABILITIES:
                    errors.append(f"Unknown capability: {cap} (valid: {VALID_CAPABILITIES})")
    
    # Check URL format
    if "url" in card:
        url = card["url"]
        if not (url.startswith("http://") or url.startswith("https://")):
            errors.append(f"url must start with http:// or https://, got: {url}")
    
    if errors:
        return False, "; ".join(errors)
    return True, "OK"

def main():
    all_ok = True
    for path in AGENT_CARD_PATHS:
        ok, msg = validate_card(path)
        status = "PASS" if ok else "FAIL"
        print(f"[{status}] {path}: {msg}")
        if not ok:
            all_ok = False
    sys.exit(0 if all_ok else 1)

if __name__ == "__main__":
    main()
