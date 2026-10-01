import algosdk from 'algosdk';
import { readFile, open, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { NETWORKS } from '../config.mjs';

const [role, chain = 'mainnet', action] = process.argv.slice(2);
if(!['merchant','canary-payer'].includes(role)||!NETWORKS[chain]||![undefined,'--execute-once'].includes(action))throw new Error('Usage: node scripts/opt-in.mjs merchant|canary-payer mainnet|testnet [--execute-once]');
const directory=`${homedir()}/.config/agent-bounties/algorand-challenge`;
const wallet=JSON.parse(await readFile(`${directory}/${role}.json`,'utf8'));
const network=NETWORKS[chain];const algod=new algosdk.Algodv2('',network.algod,'');
const account=await algod.accountInformation(wallet.address).do();
if(account.assets?.some(a=>String(a.assetId)===network.asset)){console.log(JSON.stringify({role,chain,address:wallet.address,alreadyOptedIn:true}));process.exit(0);}
const plan={role,chain,address:wallet.address,asset:network.asset,type:'zero-amount self-transfer for USDC opt-in',feeMicroAlgo:1000,balanceMicroAlgo:String(account.amount),requiredBalanceMicroAlgo:String((account.minBalance||100000n)+101000n)};
console.log(JSON.stringify(plan));
if(!action)process.exit(0);
if(account.amount<BigInt(plan.requiredBalanceMicroAlgo))throw new Error('Fund the account with ALGO first; no transaction signed');
const params=await algod.getTransactionParams().do();
if(`algorand:${Buffer.from(params.genesisHash).toString('base64')}`!==network.network)throw new Error('Unexpected chain genesis');
if(params.minFee>1000n)throw new Error('Fee exceeds the 0.001 ALGO bound');
const txn=algosdk.makeAssetTransferTxnWithSuggestedParamsFromObject({sender:wallet.address,receiver:wallet.address,amount:0,assetIndex:BigInt(network.asset),suggestedParams:{...params,fee:1000,flatFee:true}});
const key=algosdk.mnemonicToSecretKey(wallet.mnemonic);if(key.addr.toString()!==wallet.address)throw new Error('Wallet backup mismatch');
const journal=`${directory}/opt-in-${chain}-${role}.json`;
const record={...plan,txId:txn.txID(),status:'prepared_do_not_resend_without_reconciliation'};
const f=await open(journal,'wx',0o600);await f.writeFile(JSON.stringify(record,null,2));await f.close();
await algod.sendRawTransaction(txn.signTxn(key.sk)).do();
const confirmed=await algosdk.waitForConfirmation(algod,txn.txID(),8);
await writeFile(journal,JSON.stringify({...record,status:'confirmed',confirmedRound:String(confirmed.confirmedRound)},null,2),{mode:0o600});
console.log(JSON.stringify({txId:txn.txID(),confirmedRound:String(confirmed.confirmedRound)}));
