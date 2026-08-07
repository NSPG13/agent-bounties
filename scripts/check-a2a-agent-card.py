#!/usr/bin/env python3
"""A2A Agent Card schema validator — full compliance check."""
import json, sys, os, urllib.request

# Schema: A2A Protocol v0.3 minimal Agent Card
REQUIRED_TOP = {"name", "description", "url", "version"}
REQUIRED_SKILL = {"id", "name", "description"}
VALID_SECURITY = {"none", "oauth2", "apiKey", "bearerToken"}
CARD_PATHS = [
    ".well-known/agent-card.json",
    "site/.well-known/agent-card.json",
]

def load_card(path):
    if os.path.exists(path):
        with open(path) as f:
            return json.load(f), path
    return None, path

def validate_card(card, path):
    errors = []
    warnings = []
    
    # Top-level required fields
    for field in REQUIRED_TOP:
        if field not in card:
            errors.append(f"Missing required field: {field}")
    
    # Version must be semver-like
    ver = card.get("version", "")
    parts = ver.split(".")
    if len(parts) < 2:
        errors.append(f"version '{ver}' is not semver (need at least MAJOR.MINOR)")
    
    # URL validation
    url = card.get("url", "")
    if url and not (url.startswith("http://") or url.startswith("https://")):
        errors.append(f"url '{url}' is not a valid HTTP URL")
    
    # Provider validation
    provider = card.get("provider", {})
    if isinstance(provider, dict):
        org = provider.get("organization", "")
        if org and not isinstance(org, str):
            errors.append("provider.organization must be a string")
    
    # Skills validation
    skills = card.get("skills", [])
    if isinstance(skills, list):
        seen_ids = set()
        for i, skill in enumerate(skills):
            if not isinstance(skill, dict):
                errors.append(f"skills[{i}] is not an object")
                continue
            for field in REQUIRED_SKILL:
                if field not in skill:
                    errors.append(f"skills[{i}].{field} is missing")
            sid = skill.get("id", "")
            if sid in seen_ids:
                errors.append(f"Duplicate skill id: {sid}")
            seen_ids.add(sid)
            tags = skill.get("tags", [])
            if isinstance(tags, list) and len(tags) == 0:
                warnings.append(f"skills[{i}] has empty tags — consider adding descriptors")
    else:
        errors.append("skills must be an array")
    
    # Security schemes
    schemes = card.get("securitySchemes", [])
    if isinstance(schemes, list):
        for i, scheme in enumerate(schemes):
            if isinstance(scheme, dict):
                stype = scheme.get("type", "")
                if stype not in VALID_SECURITY:
                    errors.append(f"securitySchemes[{i}].type '{stype}' not in {VALID_SECURITY}")
    
    # Service endpoint
    endpoint = card.get("serviceEndpoint", "")
    if endpoint:
        if not (endpoint.startswith("http://") or endpoint.startswith("https://")):
            errors.append(f"serviceEndpoint '{endpoint}' is not a valid URL")
    
    # Capabilities
    caps = card.get("capabilities", {})
    if isinstance(caps, dict):
        streaming = caps.get("streaming", False)
        if not isinstance(streaming, bool):
            warnings.append("capabilities.streaming should be boolean")
        push_notifications = caps.get("pushNotifications", False)
        if not isinstance(push_notifications, bool):
            warnings.append("capabilities.pushNotifications should be boolean")
    
    return errors, warnings

def main():
    all_ok = True
    total_errors = 0
    total_warnings = 0
    
    for card_path in CARD_PATHS:
        card, path = load_card(card_path)
        if card is None:
            print(f"SKIP {path}: file not found (may be fine)")
            continue
        
        errors, warnings = validate_card(card, path)
        
        if errors:
            all_ok = False
            total_errors += len(errors)
            print(f"FAIL {path}: {len(errors)} error(s)")
            for e in errors:
                print(f"  - {e}")
        else:
            print(f"PASS {path}")
        
        if warnings:
            total_warnings += len(warnings)
            for w in warnings:
                print(f"  WARN: {w}")
        
        # Summary stats
        skills_count = len(card.get("skills", []))
        schemes_count = len(card.get("securitySchemes", []))
        print(f"  Stats: {skills_count} skills, {schemes_count} security schemes, version={card.get('version','?')}")
    
    # Also check the fixture matches the live card
    fixture_path = "fixtures/a2a-agent-card.json"
    if os.path.exists(fixture_path):
        with open(fixture_path) as f:
            fixture = json.load(f)
        # Compare key fields
        for live_path in CARD_PATHS:
            if os.path.exists(live_path):
                with open(live_path) as f:
                    live = json.load(f)
                if fixture.get("name") != live.get("name"):
                    print(f"WARN: fixture name '{fixture.get('name')}' != '{live.get('name')}' in {live_path}")
                if fixture.get("version") != live.get("version"):
                    print(f"WARN: fixture version mismatch")
    
    print(f"\nTotal: {total_errors} error(s), {total_warnings} warning(s)")
    sys.exit(0 if all_ok else 1)

if __name__ == '__main__':
    main()
