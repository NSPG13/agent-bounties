#!/usr/bin/env python3
"""OpenHands execute hook — runs paid work in sandbox."""
import json, os, sys, subprocess, time

def load_evidence():
    """Load the evidence collected by the claim hook."""
    evidence_path = os.path.join(os.path.dirname(__file__), '..', 'evidence.json')
    if os.path.exists(evidence_path):
        with open(evidence_path) as f:
            return json.load(f)
    return {}

def execute_work(bounty):
    """Execute the paid work for a bounty."""
    bounty_type = bounty.get('type', 'unknown')
    task = bounty.get('task', {})
    
    result = {
        'bountyId': bounty.get('bountyId'),
        'startedAt': time.time(),
        'type': bounty_type,
        'outputs': {},
        'errors': []
    }
    
    # Clone the target repo if specified
    repo = task.get('repo', '')
    if repo:
        try:
            subprocess.run(['git', 'clone', f'https://github.com/{repo}', '/tmp/work'],
                         check=True, capture_output=True, timeout=120, text=True)
            result['outputs']['repo_cloned'] = True
        except subprocess.CalledProcessError as e:
            result['errors'].append(f'clone failed: {e.stderr[:200]}')
            result['outputs']['repo_cloned'] = False
    
    # Run tests if test command specified
    test_cmd = task.get('testCommand', '')
    if test_cmd:
        try:
            proc = subprocess.run(test_cmd.split(), capture_output=True,
                                timeout=300, text=True, cwd='/tmp/work')
            result['outputs']['test_exit_code'] = proc.returncode
            result['outputs']['test_stdout'] = proc.stdout[:500]
            if proc.returncode != 0:
                result['errors'].append(f'test failed with code {proc.returncode}')
        except subprocess.TimeoutExpired:
            result['errors'].append('test timed out after 300s')
    
    result['completedAt'] = time.time()
    result['duration_seconds'] = result['completedAt'] - result['startedAt']
    return result

def main():
    evidence = load_evidence()
    if not evidence:
        print('No evidence found — nothing to execute')
        sys.exit(0)
    
    bounties = evidence.get('bounties', [])
    results = []
    
    for bounty in bounties:
        r = execute_work(bounty)
        results.append(r)
        status = 'OK' if not r['errors'] else f'ERRORS: {len(r["errors"])}'
        print(f'  #{r["bountyId"]}: {status} ({r["duration_seconds"]:.1f}s)')
    
    # Save results
    results_path = os.path.join(os.path.dirname(__file__), '..', 'results.json')
    with open(results_path, 'w') as f:
        json.dump(results, f, indent=2)
    
    failed = sum(1 for r in results if r['errors'])
    print(f'Done: {len(results)} bounties, {failed} failed')
    sys.exit(1 if failed > 0 else 0)

if __name__ == '__main__':
    main()
