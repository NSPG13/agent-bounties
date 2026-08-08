#!/usr/bin/env python3
"""Validate A2A Agent Card against the schema."""
import json,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parent.parent
card_path=ROOT/'fixtures'/'a2a-agent-card.json'
card=json.loads(card_path.read_text())
errors=[]
if card.get('name')!='Agent Bounties':errors.append('name')
if not card.get('description'):errors.append('description')
if not card.get('version'):errors.append('version')
if not card.get('supportedInterfaces'):errors.append('supportedInterfaces')
if not card.get('capabilities'):errors.append('capabilities')
if not card.get('defaultInputModes'):errors.append('defaultInputModes')
if not card.get('defaultOutputModes'):errors.append('defaultOutputModes')
for i,iface in enumerate(card.get('supportedInterfaces',[])):
 if iface.get('protocolVersion')!='1.0':errors.append(f'iface[{i}].protocolVersion')
 if not iface.get('protocolBinding'):errors.append(f'iface[{i}].protocolBinding')
if errors:
 print(f'FAIL: {errors}')
 sys.exit(1)
print(f'PASS: Agent Card valid ({len(card.get("skills",[]))} skills, {len(card.get("supportedInterfaces",[]))} interfaces)')
