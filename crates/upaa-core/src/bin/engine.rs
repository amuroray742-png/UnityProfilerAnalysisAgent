//! JSON-lines host protocol; stdout contains protocol records only.
use std::sync::{Arc,atomic::{AtomicBool,Ordering}};
use tokio::io::{AsyncBufReadExt,AsyncReadExt,AsyncWriteExt,BufReader};
use serde_json::{json,Value};
use upaa_core::engine::{Engine,Request};
#[tokio::main]
async fn main() {
 let cancel=Arc::new(AtomicBool::new(false));let engine=Arc::new(tokio::sync::Mutex::new(Engine::new(cancel.clone())));
 let output=Arc::new(tokio::sync::Mutex::new(tokio::io::stdout()));let mut input=BufReader::new(tokio::io::stdin());let mut tasks=tokio::task::JoinSet::new();
 loop {
  let mut line=String::new();let read=(&mut input).take(65537).read_line(&mut line).await;
  if !matches!(read,Ok(n) if n>0) {break;}
  if line.len()>65536 || !line.ends_with('\n') {break;}
  let value:Value=match serde_json::from_str(&line){Ok(v)=>v,Err(_)=>break};let id=value["id"].clone();
  if value["request"]["op"]=="cancel" {cancel.store(true,Ordering::SeqCst);let mut out=output.lock().await;let _=out.write_all(format!("{}\n",json!({"id":id,"result":{"cancelled":true}})).as_bytes()).await;let _=out.flush().await;continue;}
  let request=serde_json::from_value::<Request>(value["request"].clone());let engine=engine.clone();let output=output.clone();let cancel=cancel.clone();
  tasks.spawn(async move {let mut engine=engine.lock().await;cancel.store(false,Ordering::SeqCst);let result=match request {Ok(r)=>engine.handle(r).await,Err(e)=>Err(e.to_string())};let value=match result {Ok(v)=>json!({"id":id,"result":v}),Err(e)=>json!({"id":id,"error":e})};let mut out=output.lock().await;let _=out.write_all(format!("{value}\n").as_bytes()).await;let _=out.flush().await;});
 }
 cancel.store(true,Ordering::SeqCst);while tasks.join_next().await.is_some(){}
}
