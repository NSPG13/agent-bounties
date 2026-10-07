import hashlib

def mine_nonce(job_data, difficulty=16):
    """Mines nonce for leading-zero-work bounties."""
    nonce = 0
    target = '0' * (difficulty // 4)
    while nonce < 2**20:
        hash_val = hashlib.sha256(f"{job_data}{nonce}".encode()).hexdigest()
        if hash_val.startswith(target):
            return nonce
        nonce += 1
    return None
