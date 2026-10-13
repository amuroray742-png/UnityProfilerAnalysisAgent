//! Host-owned, read-only engine. No ACP sessions or editable optimization workspace.
use crate::{capture::{Capture, Conditions},comparison,extractor,parser,project,queries::{MetricsStore,transport::*},storage};
use serde::Deserialize;
use serde_json::{json,Value};
use std::{collections::{BTreeMap,HashMap},path::PathBuf,sync::{Arc,atomic::{AtomicBool,Ordering}}};
#[derive(Deserialize)]
#[serde(tag="op",rename_all="snake_case",deny_unknown_fields)]
pub enum Request {
    Info,
    Open { id:String, path:PathBuf, expected_hash:String, #[serde(default)] conditions:Conditions },
    PrepareProject { id:String, root:PathBuf },
    Query { id:String, tool:String, #[serde(default)] arguments:Value, #[serde(default)] editor_evidence:Option<Value> },
    Compare { a:String,b:String,range_a:Option<[usize;2]>,range_b:Option<[usize;2]>,confirmed:bool,budgets:BTreeMap<String,f64> },
    Close { id:String },
}
struct Recording { capture:Capture, store:MetricsStore, project:Option<Arc<project::ProjectScope>> }
pub struct Engine { recordings:HashMap<String,Recording>,pub cancel:Arc<AtomicBool> }
impl Engine {
 pub fn new(cancel:Arc<AtomicBool>)->Self { Self { recordings:HashMap::new(),cancel } }
 pub async fn handle(&mut self,request:Request)->Result<Value,String> {
  project::files::check(&self.cancel)?;
  match request {
   Request::Info=>Ok(json!({"protocol":1,"engine":"upaa-core","version":env!("CARGO_PKG_VERSION"),"statisticMethod":"upaa-round-index-v1","formats":["data","json"],"readOnly":true})),
   Request::Open{id,path,expected_hash,conditions}=>{
    if id.is_empty() || self.recordings.contains_key(&id) || self.recordings.len()>=4 { return Err("Duplicate/invalid capture or engine capacity reached".into()); }
    if !matches!(path.extension().and_then(|s|s.to_str()),Some("data"|"json")) {return Err("Only .data and supported .json recordings are accepted".into());}
    if project::files::linked(&std::fs::symlink_metadata(&path).map_err(|e|e.to_string())?) {return Err("Linked capture rejected".into());}
    if storage::file_hash(&path,&self.cancel)?!=expected_hash {return Err("Recording fingerprint changed".into());}
    let source=path.clone();let cancel=self.cancel.clone();let runtime=tokio::runtime::Handle::current();
    let profile=tokio::task::spawn_blocking(move||{
      if source.extension().and_then(|s|s.to_str())==Some("data") { parser::data::parse_path_cancel(&source,&mut |_,_|{},cancel) }
      else {runtime.block_on(parser::parse_file(&source))}
    }).await.map_err(|e|e.to_string())?.map_err(|e|e.to_string())?;
    project::files::check(&self.cancel)?;
    if storage::file_hash(&path,&self.cancel)?!=expected_hash {return Err("Recording changed during import".into());}
    let snapshot=extractor::extract(&profile);let store=MetricsStore::new();store.set_capture(snapshot.clone(),profile.details).await;
    let result=json!({"id":id,"sha256":expected_hash,"snapshot":run_session_summary(&store).await.map_err(|e|e.to_string())?,"statisticMethod":"upaa-round-index-v1"});
    self.recordings.insert(id.clone(),Recording { capture:Capture {id,path,hash:expected_hash,conditions,snapshot,frames:profile.frames},store,project:None });Ok(result)
   },
   Request::PrepareProject{id,root}=>{
    let r=self.recordings.get_mut(&id).ok_or("Capture not loaded")?;let cancel=self.cancel.clone();let cid=id.clone();
    let scope=tokio::task::spawn_blocking(move||project::ProjectScope::prepare(cid,root,cancel)).await.map_err(|e|e.to_string())??;
    let result=scope.context();r.project=Some(Arc::new(scope));Ok(result)
   },
   Request::Query{id,tool,arguments,editor_evidence}=>{
    let r=self.recordings.get(&id).ok_or("Capture not loaded")?;
    if storage::file_hash(&r.capture.path,&self.cancel)?!=r.capture.hash {return Err("Recording changed".into());}
    if tool.starts_with("project_") {
      if !matches!(tool.as_str(),"project_summary"|"project_files"|"project_search"|"project_read"|"project_asset"|"project_references") {return Err("Read-only project tool required".into());}
      return r.project.as_ref().ok_or("Project not prepared")?.query_with_editor(&tool,arguments,editor_evidence).await;
    }
    query(&r.store,&tool,arguments).await
   },
   Request::Compare{a,b,range_a,range_b,confirmed,budgets}=>{
    if budgets.iter().any(|(k,v)|!comparison::METRICS.contains(&k.as_str())||!v.is_finite()||*v<0.) {return Err("Invalid p95 budget".into());}
    let a=&self.recordings.get(&a).ok_or("A not loaded")?.capture;let b=&self.recordings.get(&b).ok_or("B not loaded")?.capture;
    for c in [a,b] {if storage::file_hash(&c.path,&self.cancel)?!=c.hash {return Err("Recording changed".into());}}
    let mut value=comparison::compare(a,b,range_a,range_b,confirmed,&budgets)?;value["statisticMethod"]=json!("upaa-round-index-v1");Ok(value)
   },
   Request::Close{id}=>{self.recordings.remove(&id);Ok(json!({"closed":id}))}
  }
 }
}
fn number(a:&Value,key:&str,default:usize)->Result<usize,String> {match a.get(key){None=>Ok(default),Some(v)=>v.as_u64().and_then(|v|usize::try_from(v).ok()).ok_or_else(||format!("Invalid {key}"))}}
pub async fn query(s:&MetricsStore,tool:&str,a:Value)->Result<Value,String> {
 let a=if a.is_null(){json!({})}else{a};if !a.is_object(){return Err("Arguments must be an object".into());}
 let start=number(&a,"start",0)?;let limit=number(&a,"limit",20)?;
 let index=||number(&a,"frame_index",usize::MAX).and_then(|v|if v==usize::MAX {Err("frame_index required".into())}else{Ok(v)});
 let result=match tool {
  "performance_session_summary"=>run_session_summary(s).await,
  "performance_metric_semantics"=>return Ok(metric_semantics()),
  "performance_hotspots"=>run_hotspots(s,a["area"].as_str().unwrap_or("cpu"),start,limit).await,
  "performance_frames"=>run_frames(s,start,limit).await,
  "performance_frame"=>run_frame_page(s,index()?,start,limit).await,
  "performance_cpu_hierarchy"=>run_cpu_hierarchy_page(s,index()?,a.get("thread_index").filter(|v|!v.is_null()).map(|_|number(&a,"thread_index",0)).transpose()?,start,limit,number(&a,"max_depth",8)?).await,
  "performance_analysis"=>run_analysis(s,a["focus"].as_str().unwrap_or("all")).await,
  "performance_frame_evidence"=>return s.query_source().await.map_err(|e|e.to_string())?.evidence(index()?,start,limit,a["counters_only"].as_bool().unwrap_or(false)).map_err(|e|e.to_string()),
  "performance_frame_sections"=>return s.query_source().await.map_err(|e|e.to_string())?.sections(index()?,start,limit).map_err(|e|e.to_string()),
  "performance_flow_events"=>return s.query_source().await.map_err(|e|e.to_string())?.flows(index()?,number(&a,"end_frame_index",index()?)?,a["flow_id"].as_u64().map(|n|n as u32),start,limit).map_err(|e|e.to_string()),
  _=>return Err("Unsupported read-only performance tool".into())
 };result.map_err(|e|e.to_string())
}
