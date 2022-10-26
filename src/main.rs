use anyhow::{anyhow, Result};
use chrono::prelude::*;
use cosmos_sdk_proto::cosmos::distribution::v1beta1::MsgWithdrawDelegatorReward;
use cosmos_sdk_proto::cosmos::staking::v1beta1::MsgDelegate;
use cosmos_sdk_proto::cosmwasm::wasm::v1::MsgExecuteContract;
use cosmos_sdk_proto::ibc::applications::transfer::v1::MsgTransfer;
use cosmos_sdk_proto::{cosmos::bank::v1beta1::MsgSend, traits::TypeUrl};
use once_cell::sync::OnceCell;
use phf::{phf_map, Map};
use serde::{Deserialize, Serialize};
use serde_aux::prelude::*;
use serde_with::*;
use std::str::FromStr;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    RPC_ENDPOINT
        .set("https://rpc-juno.itastakers.com".to_string())
        .map_err(|_| anyhow!("Cannot set RPC_ENDPOINT once cell"))?;
    HTTP_CLIENT
        .set(Client {
            client: reqwest::Client::new(),
            semaphore: Arc::new(Semaphore::new(5)),
        })
        .map_err(|_| anyhow!("Cannot set HTTP_CLIENT once cell"))?;

    let step = 20; // max step is 20
    if let CosmosRPCResponse::Status(status) = CosmosRPC::Status.fetch_de().await? {
        let mut h = 5372138; //status.sync_info.latest_block_height; // 5328323 contains a loop buy
        while h > 0 {
            if let CosmosRPCResponse::Blockchain(blockchain) = (CosmosRPC::Blockchain {
                min_height: (h - step),
                max_height: h,
            })
            .fetch_de()
            .await?
            {
                for height in blockchain.block_metas.iter().filter_map(|block_meta| {
                    if block_meta.num_txs > 0 {
                        Some(&block_meta.header.height)
                    } else {
                        None
                    }
                }) {
                    if let CosmosRPCResponse::Block(block) =
                        (CosmosRPC::Block { height: *height }).fetch_de().await?
                    {
                        log::info!("{height}");
                        let ec_tx = block.block.data.txs.iter().find(|tx| {
                            let tmp = tx.get_msgs();
                            let ec_msg = tmp.iter().find(|msg| {
                                if let TxMsg::ExecuteContract(ec) = msg {
                                    ec.contract == LOOP_NFT_CONTRACT
                                } else {
                                    false
                                }
                            });
                            ec_msg.is_some()
                        });
                        if ec_tx.is_some() {
                            for tx in block.block.data.txs {
                                let tmp = tx.get_msgs();
                                for tx_msg in tmp {
                                    if let TxMsg::ExecuteContract(ec) = tx_msg {
                                        let msg: LoopMsg = serde_json::from_slice(&ec.msg)
                                            .map_err(|e| {
                                                println!("{}", String::from_utf8_lossy(&ec.msg));
                                                e
                                            })?;
                                        println!(
                                            "{}: {} -> {:?}",
                                            height.clone(),
                                            ec.contract,
                                            msg
                                        );
                                    } else {
                                        // println!("{}: {:?}", height.clone(), tx_msg);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            h -= step;
        }
    }
    Ok(())
}

static RPC_ENDPOINT: OnceCell<String> = OnceCell::new();
static HTTP_CLIENT: OnceCell<Client> = OnceCell::new();
const LOOP_NFT_CONTRACT: &str = "juno1kylehdql046nep2gtdgl56sl09l7wv4q6cj44cwuzfs6wxnh4flszdmkk0";

const LEVANA_NFT: Map<&str, &str> = phf_map! {
    "rider" => "juno1uw3pxkga3dxhqs28rjgpgqkvcuedhmc7ezm2yxvvaey9yugfu3fq7ej2wv",
    "egg" => "juno1a90f8jdwm4h43yzqgj4xqzcfxt4l98ev970vwz6l9m02wxlpqd2squuv6k",
    "loot" => "juno1gmnkf4fs0qrwxdjcwngq3n2gpxm7t24g8n4hufhyx58873he85ss8q9va4",
};

enum CosmosRPC {
    Blockchain { min_height: u64, max_height: u64 },
    Block { height: u64 },
    Status,
}
impl CosmosRPC {
    async fn fetch_de(self) -> Result<CosmosRPCResponse> {
        let client_locked = HTTP_CLIENT
            .get()
            .ok_or_else(|| anyhow!("Could not get HTTP_CLIENT"))?
            .clone();
        let permit = client_locked.semaphore.acquire().await?;
        let ret = match self {
            CosmosRPC::Blockchain {
                min_height,
                max_height,
            } => {
                let url = format!(
                    "{}/blockchain?minHeight={min_height}&maxHeight={max_height}",
                    RPC_ENDPOINT.get().unwrap()
                );
                let x: BlockchainResponse_ =
                    client_locked.client.get(url).send().await?.json().await?;
                Ok(CosmosRPCResponse::Blockchain(x.result))
            }
            CosmosRPC::Block { height } => {
                let url = format!("{}/block?height={height}", RPC_ENDPOINT.get().unwrap());
                let x: BlockResponse_ = client_locked.client.get(url).send().await?.json().await?;
                Ok(CosmosRPCResponse::Block(x.result))
            }
            CosmosRPC::Status => {
                let url = format!("{}/status", RPC_ENDPOINT.get().unwrap());
                let x: StatusResponse_ = client_locked.client.get(url).send().await?.json().await?;
                Ok(CosmosRPCResponse::Status(x.result))
            }
        };
        // Force to reduce parallel connections. Is this the best place to do this?
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        drop(permit);
        ret
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct StatusResponse_ {
    result: StatusResponse,
}
#[derive(Serialize, Deserialize, Debug)]
struct StatusResponse {
    sync_info: RPCStatusResponseSyncInfo,
}

#[derive(Serialize, Deserialize, Debug)]
struct BlockResponse_ {
    result: BlockResponse,
}
#[derive(Serialize, Deserialize, Debug)]
struct BlockResponse {
    block: RPCBlockResponseBlock,
    block_id: RPCBlockchainResponseBlockId,
}

#[derive(Serialize, Deserialize, Debug)]
struct BlockchainResponse_ {
    result: BlockchainResponse,
}
#[derive(Serialize, Deserialize, Debug)]
struct BlockchainResponse {
    #[serde(deserialize_with = "deserialize_number_from_string")]
    last_height: u64,
    block_metas: Vec<RPCBlockchainResponseBlockMeta>,
}

#[derive(Serialize, Deserialize, Debug)]
enum CosmosRPCResponse {
    Blockchain(BlockchainResponse),
    Block(BlockResponse),
    Status(StatusResponse),
}

#[derive(Serialize, Deserialize, Debug)]
struct RPCStatusResponseSyncInfo {
    earliest_block_time: DateTime<Utc>,
    #[serde(deserialize_with = "deserialize_number_from_string")]
    earliest_block_height: u64,
    latest_block_time: DateTime<Utc>,
    #[serde(deserialize_with = "deserialize_number_from_string")]
    latest_block_height: u64,
}

#[derive(Serialize, Deserialize, Debug)]
struct RPCBlockResponseBlock {
    data: RPCBlockResponseBlockData,
    header: RPCBlockResponseHeader,
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug)]
struct RPCBlockResponseBlockData {
    #[serde_as(as = "Vec<DisplayFromStr>")]
    txs: Vec<Tx>,
}

#[derive(Serialize, Deserialize, Debug)]
struct RPCBlockResponseHeader {
    #[serde(deserialize_with = "deserialize_number_from_string")]
    height: u64,
    time: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, Debug)]
struct RPCBlockchainResponseBlockId {
    hash: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct RPCBlockchainResponseBlockMeta {
    #[serde(deserialize_with = "deserialize_number_from_string")]
    num_txs: u64,
    #[serde(deserialize_with = "deserialize_number_from_string")]
    block_size: u64,
    header: RPCBlockResponseHeader,
    block_id: RPCBlockchainResponseBlockId,
}

#[derive(Debug)]
struct Tx(cosmos_sdk_proto::cosmos::tx::v1beta1::Tx);
impl std::fmt::Display for Tx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use cosmos_sdk_proto::traits::MessageExt;
        let tx = self.0.clone();
        let str = base64::encode(tx.to_bytes().map_err(|_| std::fmt::Error::default())?);
        f.write_str(&str)
    }
}
impl FromStr for Tx {
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use cosmos_sdk_proto::traits::Message;
        Ok(Tx(cosmos_sdk_proto::cosmos::tx::v1beta1::Tx::decode(
            &base64::decode(s)?[..],
        )?))
    }

    type Err = anyhow::Error;
}
impl Tx {
    fn get_msgs(&self) -> Vec<TxMsg> {
        use cosmos_sdk_proto::traits::MessageExt;
        self.0
            .body
            .clone()
            .map(|body| {
                body.messages
                    .iter()
                    .filter_map(|message| match message.type_url.as_str() {
                        MsgSend::TYPE_URL => MsgSend::from_any(message).ok().map(TxMsg::Send),
                        MsgExecuteContract::TYPE_URL => MsgExecuteContract::from_any(message)
                            .ok()
                            .map(TxMsg::ExecuteContract),
                        MsgTransfer::TYPE_URL => {
                            MsgTransfer::from_any(message).ok().map(TxMsg::Transfer)
                        }
                        MsgDelegate::TYPE_URL => {
                            MsgDelegate::from_any(message).ok().map(TxMsg::Delegate)
                        }
                        MsgWithdrawDelegatorReward::TYPE_URL => {
                            MsgWithdrawDelegatorReward::from_any(message)
                                .ok()
                                .map(TxMsg::WithdrawDelegatorReward)
                        }
                        &_ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[derive(Debug)]
enum TxMsg {
    Send(MsgSend),
    ExecuteContract(MsgExecuteContract),
    Transfer(MsgTransfer),
    Delegate(MsgDelegate),
    WithdrawDelegatorReward(MsgWithdrawDelegatorReward),
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum LoopMsg {
    Withdraw(LoopWithdraw),
    Buy(LoopBuy),
    Send(LoopSend),
    Mint(LoopMint),
    Vote(LoopVote),
    Transfer(LoopTransfer),
    Claim(LoopClaim),
    AddToken(LoopAddToken),
    IncreaseAllowance(LoopIncreaseAllowance),
    AddLiquidity(LoopAddLiquidity),
    Swap(LoopSwap),
    PassThroughSwap(LoopPassThroughSwap),
    ClaimReward(LoopClaimReward),
    Spin(LoopSpin),
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct LoopWithdraw {
    nft_contract_addr: String,
    nft_token_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct LoopBuy {
    nft_contract_addr: String,
    nft_token_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct LoopSend {
    #[serde(deserialize_with = "deserialize_number_from_string")]
    amount: u64,
    contract: String,
    msg: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct LoopMint {
    #[serde(deserialize_with = "deserialize_number_from_string")]
    amount: u64,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct LoopVote {
    #[serde(deserialize_with = "deserialize_number_from_string")]
    proposal_id: u64,
    vote: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct LoopTransfer {
    recipient: String,
    #[serde(deserialize_with = "deserialize_number_from_string")]
    amount: u64,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct LoopClaim {
    #[serde(default)]
    stage: u64,
    #[serde(default, deserialize_with = "deserialize_number_from_string")]
    amount: u64,
    #[serde(default)]
    proof: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct LoopAddToken {
    input_token: String,
    #[serde(deserialize_with = "deserialize_number_from_string")]
    amount: u64,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct LoopIncreaseAllowance {
    #[serde(deserialize_with = "deserialize_number_from_string")]
    amount: u64,
    spender: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct LoopAddLiquidity {
    #[serde(deserialize_with = "deserialize_number_from_string")]
    token1_amount: u64,
    #[serde(deserialize_with = "deserialize_number_from_string")]
    max_token2: u64,
    #[serde(deserialize_with = "deserialize_number_from_string")]
    min_liquidity: u64,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct LoopSwap {
    input_token: String,
    #[serde(deserialize_with = "deserialize_number_from_string")]
    input_amount: u64,
    #[serde(deserialize_with = "deserialize_number_from_string")]
    min_output: u64,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct LoopPassThroughSwap {
    #[serde(deserialize_with = "deserialize_number_from_string")]
    output_min_token: u64,
    input_token: String,
    #[serde(deserialize_with = "deserialize_number_from_string")]
    input_token_amount: u64,
    output_amm_address: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct LoopClaimReward {
    token_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct LoopSpin {
    address: String,
    nb_of_spins: u64,
    multiplier: u64,
    nb_of_empowered_spins: u64,
    g_mode: bool,
    free_spins: String,
}

// mod

use std::sync::Arc;
use tokio::sync::Semaphore;

// This struct can be cloned to share it!
#[derive(Clone)]
struct Client {
    client: reqwest::Client,
    semaphore: Arc<Semaphore>,
}
