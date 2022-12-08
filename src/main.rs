use candid::{Decode, Encode, Nat, Principal};
use chrono::{DateTime, NaiveDateTime, SecondsFormat, Utc};
use clap::{command, Parser, Subcommand};
use ic_agent::{
    agent::http_transport::ReqwestHttpReplicaV2Transport, identity::AnonymousIdentity, Agent,
};
use ic_icrc1::{
    endpoints::{
        ArchivedTransactionRange, GetTransactionsRequest, GetTransactionsResponse, TransactionRange,
    },
    Account, Memo,
};
use serde_bytes::ByteBuf;

const SNS1_LEDGER_ID: &str = "zfcdd-tqaaa-aaaaq-aaaga-cai";

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long, default_value = SNS1_LEDGER_ID)]
    sns_ledger_id: String,
    #[arg(short, long, default_value = "https://ic0.app")]
    ic_url: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
#[command()]
enum Command {
    #[command()]
    GetLength,
    GetTransactions {
        #[arg(short, long)]
        start: u64,
        #[arg(short, long)]
        length: u64,
    },
}

#[derive(Clone, Debug)]
enum Transaction {
    Burn {
        timestamp: u64,
        from: Account,
        amount: Nat,
        memo: Option<Memo>,
        created_at_time: Option<u64>,
    },
    Mint {
        timestamp: u64,
        to: Account,
        amount: Nat,
        memo: Option<Memo>,
        created_at_time: Option<u64>,
    },
    Transfer {
        timestamp: u64,
        from: Account,
        to: Account,
        amount: Nat,
        fee: Option<Nat>,
        memo: Option<Memo>,
        created_at_time: Option<u64>,
    },
}

impl Transaction {
    pub fn get_kind(&self) -> &str {
        match self {
            Transaction::Burn { .. } => "burn",
            Transaction::Mint { .. } => "mint",
            Transaction::Transfer { .. } => "transfer",
        }
    }

    pub fn get_timestamp(&self) -> u64 {
        match self {
            Transaction::Burn { timestamp, .. } => *timestamp,
            Transaction::Mint { timestamp, .. } => *timestamp,
            Transaction::Transfer { timestamp, .. } => *timestamp,
        }
    }

    pub fn get_amount(&self) -> Nat {
        match self {
            Transaction::Burn { amount, .. } => amount.clone(),
            Transaction::Mint { amount, .. } => amount.clone(),
            Transaction::Transfer { amount, .. } => amount.clone(),
        }
    }

    pub fn get_memo(&self) -> Option<&Memo> {
        match self {
            Transaction::Burn { memo, .. } => memo.as_ref(),
            Transaction::Mint { memo, .. } => memo.as_ref(),
            Transaction::Transfer { memo, .. } => memo.as_ref(),
        }
    }

    pub fn get_created_at_time(&self) -> Option<&u64> {
        match self {
            Transaction::Burn {
                created_at_time, ..
            } => created_at_time.as_ref(),
            Transaction::Mint {
                created_at_time, ..
            } => created_at_time.as_ref(),
            Transaction::Transfer {
                created_at_time, ..
            } => created_at_time.as_ref(),
        }
    }
}

impl TryFrom<ic_icrc1::endpoints::Transaction> for Transaction {
    type Error = String;

    fn try_from(tx: ic_icrc1::endpoints::Transaction) -> Result<Self, Self::Error> {
        match tx.kind.as_str() {
            "mint" => {
                let mint = tx.mint.unwrap();
                Ok(Self::Mint {
                    timestamp: tx.timestamp,
                    to: mint.to,
                    amount: mint.amount,
                    memo: mint.memo,
                    created_at_time: mint.created_at_time,
                })
            }
            "burn" => {
                let burn = tx.burn.unwrap();
                Ok(Self::Burn {
                    timestamp: tx.timestamp,
                    from: burn.from,
                    amount: burn.amount,
                    memo: burn.memo,
                    created_at_time: burn.created_at_time,
                })
            }
            "transfer" => {
                let transfer = tx.transfer.unwrap();
                Ok(Self::Transfer {
                    timestamp: tx.timestamp,
                    from: transfer.from,
                    to: transfer.to,
                    amount: transfer.amount,
                    fee: transfer.fee,
                    memo: transfer.memo,
                    created_at_time: transfer.created_at_time,
                })
            }
            _ => Err(format!("Unknown kind {}", tx.kind)),
        }
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    run(args).await;
}

async fn print_length(agent: Agent, canister_id: Principal) {
    let req = GetTransactionsRequest {
        start: Nat::from(0 as u16),
        length: Nat::from(1 as u16),
    };
    let res = agent
        .query(&canister_id, "get_transactions")
        .with_arg(Encode!(&req).unwrap())
        .call()
        .await
        .unwrap_or_else(|e| {
            panic!(
                "Error while calling {}.get_transactions: {}",
                canister_id, e
            )
        });
    let res = Decode!(&res, GetTransactionsResponse).unwrap();
    println!("{}", res.log_length);
}

async fn print_txs(agent: Agent, canister_id: Principal, start: u64, length: u64) {
    let req = GetTransactionsRequest {
        start: Nat::from(start),
        length: Nat::from(length),
    };
    let res = agent
        .query(&canister_id, "get_transactions")
        .with_arg(Encode!(&req).unwrap())
        .call()
        .await
        .unwrap_or_else(|e| {
            panic!(
                "Error while calling {}.get_transactions: {}",
                canister_id, e
            )
        });
    let res = Decode!(&res, GetTransactionsResponse).unwrap();

    let mut idx = start;
    println!("block index|kind|datetime|from|to|amount|fee|memo|created_at_time");
    for ArchivedTransactionRange {
        callback,
        start,
        length,
    } in res.archived_transactions
    {
        let req = GetTransactionsRequest { start, length };
        let res = agent
            .query(&callback.canister_id.get().0, callback.method.clone())
            .with_arg(Encode!(&req).unwrap())
            .call()
            .await
            .unwrap_or_else(|e| {
                panic!(
                    "Error while calling {}.{}: {}",
                    callback.canister_id.get().0,
                    callback.method,
                    e
                )
            });
        let res = Decode!(&res, TransactionRange).unwrap();
        for tx in res.transactions {
            match tx.try_into() {
                Ok(tx) => println!("{}", tx_to_tsv(idx, tx)),
                Err(e) => eprintln!("Error on tx {}: {}", idx, e),
            }
            idx += 1;
        }
    }

    for tx in res.transactions {
        match tx.try_into() {
            Ok(tx) => println!("{}", tx_to_tsv(idx, tx)),
            Err(e) => eprintln!("Error on tx {}: {}", idx, e),
        }
        idx += 1;
    }
}

fn tx_to_tsv(idx: u64, tx: Transaction) -> String {
    let mut res = vec![];
    res.push(idx.to_string());
    res.push(tx.get_kind().to_string());
    res.push(timestamp_to_utc_rtc3339(&tx.get_timestamp()));
    res.push(get_from(&tx));
    res.push(get_to(&tx));
    res.push(tx.get_amount().to_string());
    res.push(get_fee(&tx));
    res.push(tx.get_memo().map_or(String::new(), memo_to_str));
    res.push(
        tx.get_created_at_time()
            .map_or(String::new(), timestamp_to_utc_rtc3339),
    );
    res.join("|")
}

fn subaccount_to_str(subaccount: [u8; 32]) -> String {
    subaccount
        .iter()
        .map(|byte| format!("{:02X}", byte))
        .collect()
}

fn account_to_str(account: &Account) -> String {
    let subaccount = account
        .subaccount
        .map(subaccount_to_str)
        .unwrap_or_default();
    format!("{} {}", account.owner, subaccount)
}

fn get_from(tx: &Transaction) -> String {
    match tx {
        Transaction::Burn { from, .. } => account_to_str(&from),
        Transaction::Mint { .. } => String::new(),
        Transaction::Transfer { from, .. } => account_to_str(&from),
    }
}

fn get_to(tx: &Transaction) -> String {
    match tx {
        Transaction::Burn { .. } => String::new(),
        Transaction::Mint { to, .. } => account_to_str(&to),
        Transaction::Transfer { to, .. } => account_to_str(&to),
    }
}

fn get_fee(tx: &Transaction) -> String {
    match tx {
        Transaction::Transfer { fee, .. } => {
            fee.as_ref().map_or(String::new(), |fee| fee.to_string())
        }
        _ => String::new(),
    }
}

fn memo_to_str(memo: &Memo) -> String {
    Into::<ByteBuf>::into(memo.clone())
        .iter()
        .map(|byte| format!("{:02X}", byte))
        .collect()
}

fn timestamp_to_utc_rtc3339(timestamp: &u64) -> String {
    let secs = timestamp / 1_000_000_000;
    let nsecs = timestamp % 1_000_000_000;
    let datetime = NaiveDateTime::from_timestamp_opt(secs as i64, nsecs as u32).unwrap();
    let datetime = DateTime::<Utc>::from_utc(datetime, Utc);
    datetime.to_rfc3339_opts(SecondsFormat::Millis, false)
}

async fn run(args: Args) {
    let canister_id = Principal::from_text(args.sns_ledger_id.clone())
        .unwrap_or_else(|e| panic!("Cannot parse Principal from {}: {}", args.sns_ledger_id, e));
    let agent = Agent::builder()
        .with_identity(AnonymousIdentity)
        .with_transport(ReqwestHttpReplicaV2Transport::create(args.ic_url).unwrap())
        .build()
        .unwrap();

    match args.command {
        Command::GetLength => print_length(agent, canister_id).await,
        Command::GetTransactions { start, length } => {
            print_txs(agent, canister_id, start, length).await
        }
    }
}
