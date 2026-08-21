(() => {
  "use strict";

  const CHAIN_ID = "0x2105";
  const OWNER = "0x884834e884d6e93462655a2820140ad03e6747bc";
  const USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";
  const FACTORY = "0xa45c6636d75fc94eec8cf6f6a34308c687e42ce4";
  const RELEASE_HASH = "0x0195f28ff1705e7613b55fbe6407092ceaba5c9c6d2b68bbf3f73558192854be";
  const BETA_RISK_HASH = "0x2ae728de7d15efcb7e4cb20fab5acb1b193f61ea5c80f05ad9c8a6ed45c76353";
  const PLAN_SHA256 = "0x9704dee0c561b28324df167adc793ae88c7ceff755561296076050ee63c1855a";
  const PER_COMPETITION = 3_040_000n;
  const TOTAL_FUNDING = 30_400_000n;
  const MINIMUM_GAS_BALANCE = 100_000_000_000_000n;
  const RELEASE_URL = "https://api.agentbounties.app/v1/base/open-competition-v2-beta3/release";
  const INVENTORY_URL = "https://api.agentbounties.app/v1/base/open-competition-v2-beta3/inventory?network=base-mainnet";
  const PROFILE = Object.freeze({
    profile_id: "structured-artifact-metric-v1",
    program_vkey: "0x00fb6d8580d8a7b7c55a0e49c0a662cf04500300ccd7d852c737da81c246fcdf",
    source_hash: "0xaeb170fc3da0f9fcc6b52927eb7e7dd7fdbef79c6332b893d10e23cb7a014cf5",
    elf_hash: "0x3bf75d57b9e43244b6a4929b8f7aec058bb8f09c725b908a3645c23f6d203fc8",
    journal_schema_hash: "0x63c02a04ca74b569649c9374b088b08d90fb1e85d2be0d1e0ca141307938fb0d",
    metric_program_hash: "0x760b8c342a91b4c215b8f102c85b696e70073a98c62a87987d2930eadbeb22b9",
  });
  const FROZEN_ENTRIES = Object.freeze([
  {
    "candidate_id": "agent-scanner-action-compatibility-v1",
    "title": "Test agent-scanner action compatibility",
    "predicted_competition": "0x3de38063d7f7c0e2b6f6e7c5984c6bc5545f1324",
    "bounty_id": "0x87d9101f8cf3df5f065e27b78b37df23b49f601863c0a94dd88804bce245208f",
    "calls": [
      {
        "to": "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
        "data": "0x095ea7b30000000000000000000000003de38063d7f7c0e2b6f6e7c5984c6bc5545f132400000000000000000000000000000000000000000000000000000000002e6300",
        "value": "0x0"
      },
      {
        "to": "0xa45c6636d75fc94eec8cf6f6a34308c687e42ce4",
        "data": "0x7058f67100000000000000000000000000000000000000000000000000000000002dc6c00000000000000000000000000000000000000000000000000000000000009c40000000000000000000000000000000000000000000000000000000006aafede80000000000000000000000000000000000000000000000000000000000278d000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000110fbfc39a4f588598b55fce747dc8dde3f1b661a9d538dc174b464d210d12a81d00fb6d8580d8a7b7c55a0e49c0a662cf04500300ccd7d852c737da81c246fcdfaeb170fc3da0f9fcc6b52927eb7e7dd7fdbef79c6332b893d10e23cb7a014cf53bf75d57b9e43244b6a4929b8f7aec058bb8f09c725b908a3645c23f6d203fc863c02a04ca74b569649c9374b088b08d90fb1e85d2be0d1e0ca141307938fb0d760b8c342a91b4c215b8f102c85b696e70073a98c62a87987d2930eadbeb22b90252ce3f899bcd79aea8bfb8cc20749365fa860800b64c95a5d81aed480f74ec01aa300a577e287a6b19e5f3a60673827a41e0a92c0d648a947032219b19732c666490c51c7b17146ded011de6fc23eab0af233813c2e29951719a17af845ad62ae728de7d15efcb7e4cb20fab5acb1b193f61ea5c80f05ad9c8a6ed45c7635300000000000000000000000000000000000000000000000000000000002e63000ffdcf322006cb7a7b049eb63311605e0b073a7e0c3d2d4f790d6e65920129502ae728de7d15efcb7e4cb20fab5acb1b193f61ea5c80f05ad9c8a6ed45c76353",
        "value": "0x0"
      }
    ]
  },
  {
    "candidate_id": "funding-custody-handoff-fixtures-v1",
    "title": "Create funding and custody handoff fixtures",
    "predicted_competition": "0xe7b0c4e39dca5068c863fec482b1637d5be4d119",
    "bounty_id": "0x84d72dc3c92354dbca325c2dc49d1c825d54617252e9967de6fa82f0ae6c2d62",
    "calls": [
      {
        "to": "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
        "data": "0x095ea7b3000000000000000000000000e7b0c4e39dca5068c863fec482b1637d5be4d11900000000000000000000000000000000000000000000000000000000002e6300",
        "value": "0x0"
      },
      {
        "to": "0xa45c6636d75fc94eec8cf6f6a34308c687e42ce4",
        "data": "0x7058f67100000000000000000000000000000000000000000000000000000000002dc6c00000000000000000000000000000000000000000000000000000000000009c40000000000000000000000000000000000000000000000000000000006aafede80000000000000000000000000000000000000000000000000000000000278d000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000110fbfc39a4f588598b55fce747dc8dde3f1b661a9d538dc174b464d210d12a81d00fb6d8580d8a7b7c55a0e49c0a662cf04500300ccd7d852c737da81c246fcdfaeb170fc3da0f9fcc6b52927eb7e7dd7fdbef79c6332b893d10e23cb7a014cf53bf75d57b9e43244b6a4929b8f7aec058bb8f09c725b908a3645c23f6d203fc863c02a04ca74b569649c9374b088b08d90fb1e85d2be0d1e0ca141307938fb0d760b8c342a91b4c215b8f102c85b696e70073a98c62a87987d2930eadbeb22b9eb4df97d583e0e15c54f33fd6fb1d8f0943591b2fa24f08762d5ca03f9b05963b6a2945401e93386d72c3098d10ff45d5fb38de9da5b713eb91f8b83a3057e7f666490c51c7b17146ded011de6fc23eab0af233813c2e29951719a17af845ad62ae728de7d15efcb7e4cb20fab5acb1b193f61ea5c80f05ad9c8a6ed45c7635300000000000000000000000000000000000000000000000000000000002e63008c6c739f958e0fe89c0a0194ce9045298c95f39be25dfa6ea1ef3149a01c7b622ae728de7d15efcb7e4cb20fab5acb1b193f61ea5c80f05ad9c8a6ed45c76353",
        "value": "0x0"
      }
    ]
  },
  {
    "candidate_id": "funding-wallet-trust-matrix-v1",
    "title": "Build the funding and wallet trust-objection matrix",
    "predicted_competition": "0xeb4740976ce0cc9a69e44e87566c214b8d997d16",
    "bounty_id": "0xaf88450513607abf7a93ed4dba8ae18936725c6c2e5b1815afa1337684eeb162",
    "calls": [
      {
        "to": "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
        "data": "0x095ea7b3000000000000000000000000eb4740976ce0cc9a69e44e87566c214b8d997d1600000000000000000000000000000000000000000000000000000000002e6300",
        "value": "0x0"
      },
      {
        "to": "0xa45c6636d75fc94eec8cf6f6a34308c687e42ce4",
        "data": "0x7058f67100000000000000000000000000000000000000000000000000000000002dc6c00000000000000000000000000000000000000000000000000000000000009c40000000000000000000000000000000000000000000000000000000006aafede80000000000000000000000000000000000000000000000000000000000278d000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000110fbfc39a4f588598b55fce747dc8dde3f1b661a9d538dc174b464d210d12a81d00fb6d8580d8a7b7c55a0e49c0a662cf04500300ccd7d852c737da81c246fcdfaeb170fc3da0f9fcc6b52927eb7e7dd7fdbef79c6332b893d10e23cb7a014cf53bf75d57b9e43244b6a4929b8f7aec058bb8f09c725b908a3645c23f6d203fc863c02a04ca74b569649c9374b088b08d90fb1e85d2be0d1e0ca141307938fb0d760b8c342a91b4c215b8f102c85b696e70073a98c62a87987d2930eadbeb22b93efbf0e6febd5d4b5b5451f7efbfc1640997c9c1a37574ac7370b46b4bd43b0965fc2ab6bae20d95cf576bab31af366775d463148f38b13ce443767276fe5ce8666490c51c7b17146ded011de6fc23eab0af233813c2e29951719a17af845ad62ae728de7d15efcb7e4cb20fab5acb1b193f61ea5c80f05ad9c8a6ed45c7635300000000000000000000000000000000000000000000000000000000002e6300e6b6d9cc08ac2414f30f909cb99231a22b18d860c48e5378f1d6225b01b4be842ae728de7d15efcb7e4cb20fab5acb1b193f61ea5c80f05ad9c8a6ed45c76353",
        "value": "0x0"
      }
    ]
  },
  {
    "candidate_id": "landing-to-funded-action-catalog-v1",
    "title": "Design the landing-to-first-funded-action experiment catalog",
    "predicted_competition": "0x6878dc923b114d4e49b0963b2b7f368989729461",
    "bounty_id": "0x0cfa1db8619cef406d2ef470a34df9fad92c816abdc5355d2e4844220d90d601",
    "calls": [
      {
        "to": "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
        "data": "0x095ea7b30000000000000000000000006878dc923b114d4e49b0963b2b7f36898972946100000000000000000000000000000000000000000000000000000000002e6300",
        "value": "0x0"
      },
      {
        "to": "0xa45c6636d75fc94eec8cf6f6a34308c687e42ce4",
        "data": "0x7058f67100000000000000000000000000000000000000000000000000000000002dc6c00000000000000000000000000000000000000000000000000000000000009c40000000000000000000000000000000000000000000000000000000006aafede80000000000000000000000000000000000000000000000000000000000278d000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000110fbfc39a4f588598b55fce747dc8dde3f1b661a9d538dc174b464d210d12a81d00fb6d8580d8a7b7c55a0e49c0a662cf04500300ccd7d852c737da81c246fcdfaeb170fc3da0f9fcc6b52927eb7e7dd7fdbef79c6332b893d10e23cb7a014cf53bf75d57b9e43244b6a4929b8f7aec058bb8f09c725b908a3645c23f6d203fc863c02a04ca74b569649c9374b088b08d90fb1e85d2be0d1e0ca141307938fb0d760b8c342a91b4c215b8f102c85b696e70073a98c62a87987d2930eadbeb22b915864dd8bd4e62c74e7062cbb2a83a53d068a01100de0f49f894853b8eb95fdf20cbb406c05f4f67be2c404c7b464c0d6f788f116c57bbac7156e8e9e56f3496666490c51c7b17146ded011de6fc23eab0af233813c2e29951719a17af845ad62ae728de7d15efcb7e4cb20fab5acb1b193f61ea5c80f05ad9c8a6ed45c7635300000000000000000000000000000000000000000000000000000000002e630093ee7af096b225a061297c5ac01f2157196a0236a73a7c4c7bb674e3c0dfd8872ae728de7d15efcb7e4cb20fab5acb1b193f61ea5c80f05ad9c8a6ed45c76353",
        "value": "0x0"
      }
    ]
  },
  {
    "candidate_id": "participant-feedback-gmv-schema-v1",
    "title": "Create the participant-feedback-to-GMV schema",
    "predicted_competition": "0x4caa808affc0551d76fa647282c5849f34a59458",
    "bounty_id": "0x12868cb580c17cfbe6e018ed5673e1fa07ba590bf1eb7e423da137fca0d59120",
    "calls": [
      {
        "to": "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
        "data": "0x095ea7b30000000000000000000000004caa808affc0551d76fa647282c5849f34a5945800000000000000000000000000000000000000000000000000000000002e6300",
        "value": "0x0"
      },
      {
        "to": "0xa45c6636d75fc94eec8cf6f6a34308c687e42ce4",
        "data": "0x7058f67100000000000000000000000000000000000000000000000000000000002dc6c00000000000000000000000000000000000000000000000000000000000009c40000000000000000000000000000000000000000000000000000000006aafede80000000000000000000000000000000000000000000000000000000000278d000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000110fbfc39a4f588598b55fce747dc8dde3f1b661a9d538dc174b464d210d12a81d00fb6d8580d8a7b7c55a0e49c0a662cf04500300ccd7d852c737da81c246fcdfaeb170fc3da0f9fcc6b52927eb7e7dd7fdbef79c6332b893d10e23cb7a014cf53bf75d57b9e43244b6a4929b8f7aec058bb8f09c725b908a3645c23f6d203fc863c02a04ca74b569649c9374b088b08d90fb1e85d2be0d1e0ca141307938fb0d760b8c342a91b4c215b8f102c85b696e70073a98c62a87987d2930eadbeb22b961dc182d3eb3455426a7383fa3d62edded58fe89cd271fbd45019817874c09b1a1f6e5483f8c27dd4d76d42d765ad429aa8b926df30d6fabdd90ff8476e8d2a9666490c51c7b17146ded011de6fc23eab0af233813c2e29951719a17af845ad62ae728de7d15efcb7e4cb20fab5acb1b193f61ea5c80f05ad9c8a6ed45c7635300000000000000000000000000000000000000000000000000000000002e6300237c74f2e9414988f0594de34fd85e64eb89b61bac147ae801591e9f9ff6ff9d2ae728de7d15efcb7e4cb20fab5acb1b193f61ea5c80f05ad9c8a6ed45c76353",
        "value": "0x0"
      }
    ]
  },
  {
    "candidate_id": "post-the-next-bounty-library-v1",
    "title": "Build the post-settlement next-bounty action library",
    "predicted_competition": "0x8e18a2df84cfeb435ef00f19f283ec01ccd06042",
    "bounty_id": "0xa9b7b803c323e4e5432d2198ce2281e4d3fb0e502a7c67bb1deaca64427ead84",
    "calls": [
      {
        "to": "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
        "data": "0x095ea7b30000000000000000000000008e18a2df84cfeb435ef00f19f283ec01ccd0604200000000000000000000000000000000000000000000000000000000002e6300",
        "value": "0x0"
      },
      {
        "to": "0xa45c6636d75fc94eec8cf6f6a34308c687e42ce4",
        "data": "0x7058f67100000000000000000000000000000000000000000000000000000000002dc6c00000000000000000000000000000000000000000000000000000000000009c40000000000000000000000000000000000000000000000000000000006aafede80000000000000000000000000000000000000000000000000000000000278d000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000110fbfc39a4f588598b55fce747dc8dde3f1b661a9d538dc174b464d210d12a81d00fb6d8580d8a7b7c55a0e49c0a662cf04500300ccd7d852c737da81c246fcdfaeb170fc3da0f9fcc6b52927eb7e7dd7fdbef79c6332b893d10e23cb7a014cf53bf75d57b9e43244b6a4929b8f7aec058bb8f09c725b908a3645c23f6d203fc863c02a04ca74b569649c9374b088b08d90fb1e85d2be0d1e0ca141307938fb0d760b8c342a91b4c215b8f102c85b696e70073a98c62a87987d2930eadbeb22b9fd18d94b62eeeb0cb0b6a6d300c4560de9a8851032e9708ea86c0309e6b6da2c8c9786727f55b3b66b60e12111106de21060103db95b70ddcfb8f70d87251c24666490c51c7b17146ded011de6fc23eab0af233813c2e29951719a17af845ad62ae728de7d15efcb7e4cb20fab5acb1b193f61ea5c80f05ad9c8a6ed45c7635300000000000000000000000000000000000000000000000000000000002e63005285e066c1ba37df661ffaac21a11bfc67eb4571ae65e9469bf51740f4116ebc2ae728de7d15efcb7e4cb20fab5acb1b193f61ea5c80f05ad9c8a6ed45c76353",
        "value": "0x0"
      }
    ]
  },
  {
    "candidate_id": "poster-activation-readiness-matrix-v1",
    "title": "Build the external poster activation readiness matrix",
    "predicted_competition": "0x663075ccd3952bf3d8b9f461224108776984cbb4",
    "bounty_id": "0x8a361b96e5e4374ccabf9c33d933bc1724cd1b4af22a3b369a922955b3b625b0",
    "calls": [
      {
        "to": "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
        "data": "0x095ea7b3000000000000000000000000663075ccd3952bf3d8b9f461224108776984cbb400000000000000000000000000000000000000000000000000000000002e6300",
        "value": "0x0"
      },
      {
        "to": "0xa45c6636d75fc94eec8cf6f6a34308c687e42ce4",
        "data": "0x7058f67100000000000000000000000000000000000000000000000000000000002dc6c00000000000000000000000000000000000000000000000000000000000009c40000000000000000000000000000000000000000000000000000000006aafede80000000000000000000000000000000000000000000000000000000000278d000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000110fbfc39a4f588598b55fce747dc8dde3f1b661a9d538dc174b464d210d12a81d00fb6d8580d8a7b7c55a0e49c0a662cf04500300ccd7d852c737da81c246fcdfaeb170fc3da0f9fcc6b52927eb7e7dd7fdbef79c6332b893d10e23cb7a014cf53bf75d57b9e43244b6a4929b8f7aec058bb8f09c725b908a3645c23f6d203fc863c02a04ca74b569649c9374b088b08d90fb1e85d2be0d1e0ca141307938fb0d760b8c342a91b4c215b8f102c85b696e70073a98c62a87987d2930eadbeb22b9bed85d6e3e527f36c9db2f4ba901bc4ba898472b440a8a4412cda6eda1bb511dfcf25a7018340ebdd736b8a3f7f6c5dac8b145d27c43124cdebc4d6c1e6eafd5666490c51c7b17146ded011de6fc23eab0af233813c2e29951719a17af845ad62ae728de7d15efcb7e4cb20fab5acb1b193f61ea5c80f05ad9c8a6ed45c7635300000000000000000000000000000000000000000000000000000000002e630059709341e226ece10b33affe4f26329bcbd1ba6d1904d4441beed067c8168af52ae728de7d15efcb7e4cb20fab5acb1b193f61ea5c80f05ad9c8a6ed45c76353",
        "value": "0x0"
      }
    ]
  },
  {
    "candidate_id": "proof-payment-recovery-corpus-v1",
    "title": "Build the proof and payment-pending recovery corpus",
    "predicted_competition": "0xb213ab86a81fafc699c9dc687ccf4a57348d97e2",
    "bounty_id": "0xe34b014ab16a8dcb3ab7890708624374faca351df798f35b5565b6bac13c7fc6",
    "calls": [
      {
        "to": "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
        "data": "0x095ea7b3000000000000000000000000b213ab86a81fafc699c9dc687ccf4a57348d97e200000000000000000000000000000000000000000000000000000000002e6300",
        "value": "0x0"
      },
      {
        "to": "0xa45c6636d75fc94eec8cf6f6a34308c687e42ce4",
        "data": "0x7058f67100000000000000000000000000000000000000000000000000000000002dc6c00000000000000000000000000000000000000000000000000000000000009c40000000000000000000000000000000000000000000000000000000006aafede80000000000000000000000000000000000000000000000000000000000278d000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000110fbfc39a4f588598b55fce747dc8dde3f1b661a9d538dc174b464d210d12a81d00fb6d8580d8a7b7c55a0e49c0a662cf04500300ccd7d852c737da81c246fcdfaeb170fc3da0f9fcc6b52927eb7e7dd7fdbef79c6332b893d10e23cb7a014cf53bf75d57b9e43244b6a4929b8f7aec058bb8f09c725b908a3645c23f6d203fc863c02a04ca74b569649c9374b088b08d90fb1e85d2be0d1e0ca141307938fb0d760b8c342a91b4c215b8f102c85b696e70073a98c62a87987d2930eadbeb22b9444a0cec2ac92263acb6502fe2f487e4e48a75e71a40fed4d3e3609075610795661b777aaf5b7695f12f00c3e31961ca08dd99d6fb906869b3270e0ce2a928b5666490c51c7b17146ded011de6fc23eab0af233813c2e29951719a17af845ad62ae728de7d15efcb7e4cb20fab5acb1b193f61ea5c80f05ad9c8a6ed45c7635300000000000000000000000000000000000000000000000000000000002e63008446fb514038f3cbf69b35f3723527ab29db0dc27585f0dc33f56e912c28c7382ae728de7d15efcb7e4cb20fab5acb1b193f61ea5c80f05ad9c8a6ed45c76353",
        "value": "0x0"
      }
    ]
  },
  {
    "candidate_id": "verifier-ready-terms-corpus-v1",
    "title": "Create a verifier-ready competition terms corpus",
    "predicted_competition": "0x6200249b7faa32679233e0b15270412f1354edaf",
    "bounty_id": "0xcce5d626ca6a7adbb0d2b76087abcb42482fb50e70330bf8ad54ac8a6f8a87a4",
    "calls": [
      {
        "to": "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
        "data": "0x095ea7b30000000000000000000000006200249b7faa32679233e0b15270412f1354edaf00000000000000000000000000000000000000000000000000000000002e6300",
        "value": "0x0"
      },
      {
        "to": "0xa45c6636d75fc94eec8cf6f6a34308c687e42ce4",
        "data": "0x7058f67100000000000000000000000000000000000000000000000000000000002dc6c00000000000000000000000000000000000000000000000000000000000009c40000000000000000000000000000000000000000000000000000000006aafede80000000000000000000000000000000000000000000000000000000000278d000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000110fbfc39a4f588598b55fce747dc8dde3f1b661a9d538dc174b464d210d12a81d00fb6d8580d8a7b7c55a0e49c0a662cf04500300ccd7d852c737da81c246fcdfaeb170fc3da0f9fcc6b52927eb7e7dd7fdbef79c6332b893d10e23cb7a014cf53bf75d57b9e43244b6a4929b8f7aec058bb8f09c725b908a3645c23f6d203fc863c02a04ca74b569649c9374b088b08d90fb1e85d2be0d1e0ca141307938fb0d760b8c342a91b4c215b8f102c85b696e70073a98c62a87987d2930eadbeb22b9a4278174fff64acefe838207bf71a88a7ecd271d35f465aeb228fe7f5df712de4de09bb36d67873191eed5d2721bbe7a70ce695296cd107b6ce083574839bc7e666490c51c7b17146ded011de6fc23eab0af233813c2e29951719a17af845ad62ae728de7d15efcb7e4cb20fab5acb1b193f61ea5c80f05ad9c8a6ed45c7635300000000000000000000000000000000000000000000000000000000002e6300d316ccfeade51c8578513877dd3f02d1a2f026965ff8a6cea2343a10acd388262ae728de7d15efcb7e4cb20fab5acb1b193f61ea5c80f05ad9c8a6ed45c76353",
        "value": "0x0"
      }
    ]
  },
  {
    "candidate_id": "wrong-mode-routing-corpus-v1",
    "title": "Build the wrong-competition-mode routing corpus",
    "predicted_competition": "0x4e60dc5fe8a8dbdb343d1969dcbb3b3b0c92d087",
    "bounty_id": "0xe02d052e0f61a5a976d0df64caaf0f1a0b5095543415cfc19531377338e6e796",
    "calls": [
      {
        "to": "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
        "data": "0x095ea7b30000000000000000000000004e60dc5fe8a8dbdb343d1969dcbb3b3b0c92d08700000000000000000000000000000000000000000000000000000000002e6300",
        "value": "0x0"
      },
      {
        "to": "0xa45c6636d75fc94eec8cf6f6a34308c687e42ce4",
        "data": "0x7058f67100000000000000000000000000000000000000000000000000000000002dc6c00000000000000000000000000000000000000000000000000000000000009c40000000000000000000000000000000000000000000000000000000006aafede80000000000000000000000000000000000000000000000000000000000278d000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000110fbfc39a4f588598b55fce747dc8dde3f1b661a9d538dc174b464d210d12a81d00fb6d8580d8a7b7c55a0e49c0a662cf04500300ccd7d852c737da81c246fcdfaeb170fc3da0f9fcc6b52927eb7e7dd7fdbef79c6332b893d10e23cb7a014cf53bf75d57b9e43244b6a4929b8f7aec058bb8f09c725b908a3645c23f6d203fc863c02a04ca74b569649c9374b088b08d90fb1e85d2be0d1e0ca141307938fb0d760b8c342a91b4c215b8f102c85b696e70073a98c62a87987d2930eadbeb22b918bf0e96eef578b7181797bf0814e351cd0dadc13adf062e439434c69e384d1520a189f7666c1e2e9162a36cab5d6bc65fd54d30df1a3086a5e009f428ab3b16666490c51c7b17146ded011de6fc23eab0af233813c2e29951719a17af845ad62ae728de7d15efcb7e4cb20fab5acb1b193f61ea5c80f05ad9c8a6ed45c7635300000000000000000000000000000000000000000000000000000000002e63006c5f9aae7a4cf4c0e49d7b1269a7c7031c07a4eacde223d9fe6c2add231f3aa72ae728de7d15efcb7e4cb20fab5acb1b193f61ea5c80f05ad9c8a6ed45c76353",
        "value": "0x0"
      }
    ]
  }
]);

  const ui = {
    status: document.querySelector("#refill-status"),
    output: document.querySelector("#refill-output"),
    button: document.querySelector("#confirm-refill"),
    candidates: document.querySelector("#refill-candidates"),
  };
  const discoveredProviders = [];
  let provider = null;

  function normalize(value) {
    return String(value || "").toLowerCase();
  }

  function setStatus(label, message, tone) {
    ui.status.textContent = label;
    ui.status.dataset.tone = tone || "";
    ui.output.textContent = message;
    ui.output.dataset.tone = tone || "";
  }

  function fail(message) {
    throw new Error(message);
  }

  function word(data, index) {
    const start = 10 + (index * 64);
    return "0x" + data.slice(start, start + 64);
  }

  function uintWord(data, index) {
    return BigInt(word(data, index));
  }

  function addressFromWord(value) {
    return "0x" + String(value).slice(-40).toLowerCase();
  }

  function addressWord(address) {
    return normalize(address).replace(/^0x/, "").padStart(64, "0");
  }

  function balanceOfCalldata(address) {
    return "0x70a08231" + addressWord(address);
  }

  function allowanceCalldata(owner, spender) {
    return "0xdd62ed3e" + addressWord(owner) + addressWord(spender);
  }

  function calls() {
    return FROZEN_ENTRIES.flatMap((entry) => entry.calls.map((call) => ({
      to: call.to,
      data: call.data,
      value: "0x0",
    })));
  }

  async function sha256Hex(text) {
    const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
    return "0x" + Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join("");
  }

  async function validateFrozenPlan(nowSeconds = Math.floor(Date.now() / 1000)) {
    if (await sha256Hex(JSON.stringify(FROZEN_ENTRIES)) !== PLAN_SHA256) {
      fail("The frozen batch content hash does not match. No wallet request was made.");
    }
    if (FROZEN_ENTRIES.length !== 10) fail("The frozen batch must contain exactly ten opportunities.");
    const candidateIds = new Set();
    const contracts = new Set();
    const bountyIds = new Set();
    for (const entry of FROZEN_ENTRIES) {
      if (candidateIds.has(entry.candidate_id) || contracts.has(entry.predicted_competition) || bountyIds.has(entry.bounty_id)) {
        fail("The frozen batch contains a duplicate candidate, contract, or bounty ID.");
      }
      candidateIds.add(entry.candidate_id);
      contracts.add(entry.predicted_competition);
      bountyIds.add(entry.bounty_id);
      if (!Array.isArray(entry.calls) || entry.calls.length !== 2) fail("Each opportunity must have exactly two wallet calls.");
      const approval = entry.calls[0];
      const creation = entry.calls[1];
      if (normalize(approval.to) !== USDC || !normalize(approval.data).startsWith("0x095ea7b3")) {
        fail("A frozen USDC approval is malformed.");
      }
      if (addressFromWord(word(approval.data, 0)) !== normalize(entry.predicted_competition)) {
        fail("A frozen approval spender does not match its predicted competition.");
      }
      if (uintWord(approval.data, 1) !== PER_COMPETITION) fail("A frozen approval is not exactly 3.04 USDC.");
      if (normalize(creation.to) !== FACTORY || !normalize(creation.data).startsWith("0x7058f671")) {
        fail("A frozen creation call does not target the reviewed factory function.");
      }
      if (creation.data.length !== 1290) fail("A frozen creation call has an unexpected ABI length.");
      if (uintWord(creation.data, 0) !== 3_000_000n || uintWord(creation.data, 1) !== 40_000n) {
        fail("Frozen competition rewards differ from 3.00 + 0.04 USDC.");
      }
      if (uintWord(creation.data, 2) <= BigInt(nowSeconds + 86_400)) fail("A frozen funding deadline is too close or expired.");
      if (uintWord(creation.data, 3) !== 2_592_000n || uintWord(creation.data, 4) !== 0n) {
        fail("A frozen proof window or winner mode changed.");
      }
      if (word(creation.data, 8) !== PROFILE.program_vkey
          || word(creation.data, 9) !== PROFILE.source_hash
          || word(creation.data, 10) !== PROFILE.elf_hash
          || word(creation.data, 11) !== PROFILE.journal_schema_hash
          || word(creation.data, 12) !== PROFILE.metric_program_hash) {
        fail("A frozen creation does not use the reviewed structured-artifact profile.");
      }
      if (word(creation.data, 16) !== BETA_RISK_HASH
          || uintWord(creation.data, 17) !== PER_COMPETITION
          || word(creation.data, 19) !== BETA_RISK_HASH) {
        fail("A frozen creation changed its funding or acknowledged risk.");
      }
    }
    if (calls().length !== 20) fail("The atomic batch must contain exactly twenty calls.");
    return true;
  }

  function projection(item) {
    return item && item.record && item.record.projection;
  }

  function qualifyingActive(items) {
    return (Array.isArray(items) ? items : []).filter((item) => {
      const value = projection(item);
      return value
        && value.state === "active"
        && Number(value.funded_amount) >= Number(value.solver_reward) + Number(value.keeper_reward);
    });
  }

  function plannedActive(items) {
    const byContract = new Map((Array.isArray(items) ? items : []).map((item) => [
      normalize(projection(item) && projection(item).competition),
      item,
    ]));
    return FROZEN_ENTRIES.filter((entry) => {
      const item = byContract.get(normalize(entry.predicted_competition));
      const value = projection(item);
      return value
        && normalize(value.bounty_id) === normalize(entry.bounty_id)
        && normalize(value.creator) === OWNER
        && value.state === "active"
        && Number(value.funded_amount) === Number(PER_COMPETITION)
        && Number(value.solver_reward) === 3_000_000
        && Number(value.keeper_reward) === 40_000;
    }).length;
  }

  async function fetchJson(url) {
    const response = await fetch(url, { cache: "no-store" });
    if (!response.ok) fail("Live evidence returned HTTP " + response.status + ".");
    return response.json();
  }

  function validateRelease(document) {
    const release = document && document.release;
    const agreement = document && document.indexer_agreement;
    if (document.activation_state !== "public_beta"
        || !release
        || release.release_hash !== RELEASE_HASH
        || normalize(release.factory_contract) !== FACTORY
        || normalize(release.settlement_token) !== USDC
        || release.public_creation_enabled !== true
        || release.proof_broker_enabled !== true) {
      fail("The live Base release no longer matches the reviewed release.");
    }
    if (!agreement || agreement.agrees !== true || normalize(agreement.factory_contract) !== FACTORY) {
      fail("Primary and shadow indexers do not agree. No wallet request was made.");
    }
    const observedAt = Date.parse(agreement.observed_at);
    const age = Date.now() - observedAt;
    if (!Number.isFinite(observedAt) || age > 600_000 || age < -120_000) {
      fail("Indexer agreement is stale or future-dated. No wallet request was made.");
    }
    const profile = (release.metric_programs || []).find((item) => item.profile_id === PROFILE.profile_id);
    if (!profile
        || profile.classification !== "reviewed"
        || normalize(profile.program_vkey) !== PROFILE.program_vkey
        || normalize(profile.source_hash) !== PROFILE.source_hash
        || normalize(profile.elf_hash) !== PROFILE.elf_hash
        || normalize(profile.journal_schema_hash) !== PROFILE.journal_schema_hash
        || normalize(profile.metric_program_hash) !== PROFILE.metric_program_hash) {
      fail("The reviewed structured-artifact proof profile changed.");
    }
    return release;
  }

  function validateInventory(document) {
    if (!document
        || document.protocol_version !== "agent-bounties/open-competition-v2-beta3"
        || normalize(document.factory_contract) !== FACTORY
        || document.network !== "base-mainnet"
        || !Array.isArray(document.competitions)) {
      fail("The canonical inventory response is malformed.");
    }
    return document.competitions;
  }

  function recordProvider(event) {
    const detail = event && event.detail;
    if (!detail || !detail.provider || typeof detail.provider.request !== "function") return;
    if (!discoveredProviders.some((item) => item.provider === detail.provider)) discoveredProviders.push(detail);
  }

  function selectProvider() {
    const injected = [];
    if (window.ethereum && Array.isArray(window.ethereum.providers)) injected.push(...window.ethereum.providers);
    if (window.ethereum) injected.push(window.ethereum);
    const candidates = [...discoveredProviders.map((item) => item.provider), ...injected]
      .filter((item, index, all) => item && all.indexOf(item) === index);
    const metamask = candidates.find((item) => item.isMetaMask && !item.isCoinbaseWallet);
    if (!metamask) fail("MetaMask was not found. Open this page in the browser where the owner wallet is installed.");
    return metamask;
  }

  async function wallet(method, params = []) {
    if (!provider) provider = selectProvider();
    return provider.request({ method, params });
  }

  async function connectOwner() {
    const accounts = await wallet("eth_requestAccounts");
    const account = normalize(accounts && accounts[0]);
    if (account !== OWNER) fail("Connected account " + (account || "(none)") + " is not the required owner wallet.");
    let chainId = normalize(await wallet("eth_chainId"));
    if (chainId !== CHAIN_ID) {
      await wallet("wallet_switchEthereumChain", [{ chainId: CHAIN_ID }]);
      chainId = normalize(await wallet("eth_chainId"));
    }
    if (chainId !== CHAIN_ID) fail("MetaMask did not switch to Base mainnet.");
    const freshAccounts = await wallet("eth_accounts");
    if (normalize(freshAccounts && freshAccounts[0]) !== OWNER) fail("The active owner account changed during preflight.");
    return account;
  }

  function atomicCapability(capabilities) {
    const chain = capabilities && Object.entries(capabilities)
      .find(([key]) => normalize(key) === CHAIN_ID);
    return chain && chain[1] && chain[1].atomic && chain[1].atomic.status;
  }

  async function preflightWallet(account, release, inventory) {
    const exactAlreadyActive = plannedActive(inventory);
    if (exactAlreadyActive === 10) return { alreadyActive: true };
    const active = qualifyingActive(inventory).length;
    if (active !== 0 || exactAlreadyActive !== 0) {
      fail("Inventory changed after this exact ten-opportunity batch was frozen. Stop for deterministic replanning.");
    }

    const capabilities = await wallet("wallet_getCapabilities", [account, [CHAIN_ID]]);
    const atomicStatus = atomicCapability(capabilities);
    if (atomicStatus !== "supported" && atomicStatus !== "ready") {
      fail("MetaMask does not report atomic call support on Base. Sequential transactions are intentionally disabled.");
    }

    const predicted = FROZEN_ENTRIES.map((entry) => entry.predicted_competition);
    const [ethHex, usdcHex, factoryCode, ...contractState] = await Promise.all([
      wallet("eth_getBalance", [OWNER, "latest"]),
      wallet("eth_call", [{ to: USDC, data: balanceOfCalldata(OWNER) }, "latest"]),
      wallet("eth_getCode", [FACTORY, "latest"]),
      ...predicted.flatMap((contract) => [
        wallet("eth_getCode", [contract, "latest"]),
        wallet("eth_call", [{ to: USDC, data: allowanceCalldata(OWNER, contract) }, "latest"]),
      ]),
    ]);
    const ethBalance = BigInt(ethHex);
    const usdcBalance = BigInt(usdcHex);
    if (usdcBalance < TOTAL_FUNDING) fail("Owner balance is below the exact 30.40 USDC requirement.");
    if (ethBalance < MINIMUM_GAS_BALANCE) fail("Owner Base ETH balance is below the conservative 0.0001 ETH gas minimum.");
    if (!window.AgentBountiesEvm || window.AgentBountiesEvm.keccak256Hex(factoryCode) !== release.factory_runtime_code_hash) {
      fail("Factory runtime bytecode does not match the live reviewed release.");
    }
    for (let index = 0; index < predicted.length; index += 1) {
      const code = normalize(contractState[index * 2]);
      const allowance = BigInt(contractState[(index * 2) + 1]);
      if (code !== "0x" && code !== "0x0") fail("A predicted competition address is already occupied.");
      if (allowance !== 0n) fail("A predicted competition already has a nonzero owner allowance.");
    }
    return { alreadyActive: false, ethBalance, usdcBalance };
  }

  async function livePreflight() {
    const [releaseDocument, inventoryDocument] = await Promise.all([
      fetchJson(RELEASE_URL),
      fetchJson(INVENTORY_URL),
    ]);
    const release = validateRelease(releaseDocument);
    const inventory = validateInventory(inventoryDocument);
    return { release, inventory };
  }

  async function waitForCallsStatus(id) {
    for (let attempt = 0; attempt < 240; attempt += 1) {
      const status = await wallet("wallet_getCallsStatus", [id]);
      const code = Number(status && status.status);
      if (code === 200) return status;
      if (code >= 400) fail("The atomic batch failed with wallet status " + code + ".");
      if (attempt % 10 === 0) setStatus("Confirming on Base", "Atomic batch accepted. Waiting for confirmed receipts…", "pending");
      await new Promise((resolve) => setTimeout(resolve, 2_000));
    }
    fail("Timed out waiting for the atomic wallet batch.");
  }

  async function waitForCanonicalActivation() {
    for (let attempt = 0; attempt < 300; attempt += 1) {
      const inventoryDocument = await fetchJson(INVENTORY_URL);
      const inventory = validateInventory(inventoryDocument);
      const count = plannedActive(inventory);
      if (count === 10) return inventory;
      setStatus("Canonicalizing " + count + "/10", "The batch is confirmed. Waiting for all ten funded opportunities to reach the canonical safe-block inventory…", "pending");
      await new Promise((resolve) => setTimeout(resolve, 2_000));
    }
    fail("The batch confirmed, but canonical activation is still pending. Do not resubmit; reconcile the call-batch receipts and inventory.");
  }

  async function execute() {
    ui.button.disabled = true;
    try {
      setStatus("Inspecting", "Verifying the frozen batch and fresh canonical evidence. No wallet request yet…", "pending");
      await validateFrozenPlan();
      const account = await connectOwner();
      const live = await livePreflight();
      const walletState = await preflightWallet(account, live.release, live.inventory);
      if (walletState.alreadyActive) {
        setStatus("Already restored", "All ten reviewed opportunities are already canonically active. No wallet request was made.", "success");
        return;
      }

      setStatus("Awaiting wallet", "All gates passed. Confirm the one atomic 30.40-USDC batch in MetaMask.", "pending");
      const response = await wallet("wallet_sendCalls", [{
        version: "2.0.0",
        from: OWNER,
        chainId: CHAIN_ID,
        atomicRequired: true,
        calls: calls(),
      }]);
      const id = typeof response === "string" ? response : response && response.id;
      if (!id) fail("MetaMask returned no atomic call-batch identifier.");
      localStorage.setItem("agentbounties.marketplace-refill.call-batch-id", id);
      const status = await waitForCallsStatus(id);
      if (status.atomic !== true) fail("MetaMask did not confirm atomic execution.");
      const receipts = Array.isArray(status.receipts) ? status.receipts : [];
      if (!receipts.length || receipts.some((receipt) => Number.parseInt(receipt.status, 16) !== 1)) {
        fail("The atomic call status did not include successful receipts.");
      }
      await waitForCanonicalActivation();
      const hashes = receipts.map((receipt) => receipt.transactionHash).filter(Boolean);
      localStorage.removeItem("agentbounties.marketplace-refill.call-batch-id");
      setStatus(
        "10/10 active",
        "Canonical activation confirmed for all ten funded opportunities."
          + (hashes.length ? "\nBase transaction: https://base.blockscout.com/tx/" + hashes[0] : ""),
        "success",
      );
    } catch (error) {
      const message = error && error.code === 4001
        ? "The wallet request was rejected. Nothing was spent."
        : (error && error.message) || String(error);
      setStatus("Stopped safely", message, "error");
      ui.button.disabled = false;
    }
  }

  function render() {
    ui.candidates.replaceChildren(...FROZEN_ENTRIES.map((entry) => {
      const item = document.createElement("li");
      item.append(document.createTextNode(entry.title));
      const code = document.createElement("code");
      code.textContent = entry.predicted_competition;
      item.append(code);
      return item;
    }));
  }

  async function initialReadiness() {
    render();
    try {
      await validateFrozenPlan();
      const live = await livePreflight();
      const exact = plannedActive(live.inventory);
      if (exact === 10) {
        setStatus("Already restored", "All ten reviewed opportunities are already canonically active.", "success");
        ui.button.disabled = true;
        return;
      }
      if (qualifyingActive(live.inventory).length !== 0 || exact !== 0) {
        fail("Inventory changed; this exact batch must be replanned before confirmation.");
      }
      setStatus("Ready to confirm", "Frozen batch and fresh canonical release evidence match. Click once to connect, recheck, and open MetaMask.", "success");
    } catch (error) {
      setStatus("Not ready", (error && error.message) || String(error), "error");
      ui.button.disabled = true;
    }
  }

  window.addEventListener("eip6963:announceProvider", recordProvider);
  window.dispatchEvent(new Event("eip6963:requestProvider"));
  ui.button.addEventListener("click", execute);
  window.AgentBountiesMarketplaceRefill = Object.freeze({
    FROZEN_ENTRIES,
    PLAN_SHA256,
    qualifyingActive,
    plannedActive,
    validateFrozenPlan,
  });
  initialReadiness();
})();

