import algosdk from 'algosdk';
import { mkdir, open, readFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { join } from 'node:path';

const directory = join(homedir(), '.config', 'agent-bounties', 'algorand-challenge');
await mkdir(directory, { recursive: true, mode: 0o700 });
for (const role of ['merchant', 'canary-payer']) {
  const path = join(directory, `${role}.json`);
  let wallet;
  try { wallet = JSON.parse(await readFile(path, 'utf8')); }
  catch (error) {
    if (error.code !== 'ENOENT') throw error;
    const account = algosdk.generateAccount();
    wallet = { role, address: account.addr.toString(), mnemonic: algosdk.secretKeyToMnemonic(account.sk), createdAt: new Date().toISOString() };
    const file = await open(path, 'wx', 0o600);
    try { await file.writeFile(JSON.stringify(wallet, null, 2) + '\n'); } finally { await file.close(); }
  }
  console.log(JSON.stringify({ role, address: wallet.address, privateBackupPath: path }));
}
