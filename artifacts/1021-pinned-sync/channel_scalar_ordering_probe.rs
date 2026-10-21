//! 审查探针：期望旧频道快照不覆盖更新；当前生产缺少revision，因此应红。
#[path = "../../helix-im/tests/common.rs"]
mod common;
use common::*;
use helix_core::{Effect, Tick};
use helix_core::tick::{InboundBytes, PortOutcome};
use helix_core::effect::{GetSpec,SqlValue};
use helix_core::ports::Storage;
use helix_driver_host::{HostStorage,execute_storage_ops_atomic,execute_storage_ops};
use serde_json::json;
/// 公开increment写集延迟至在线新名称持久化之后执行。
#[tokio::test]
async fn old_increment_must_not_overwrite_new_channel_name() {
    let db=HostStorage::open_sqlite_url("file:channel_scalar_probe?mode=memory&cache=shared").await.unwrap();
    for sql in helix_im::IM_SCHEMA {db.execute_raw(sql).await.unwrap();}
    db.execute_raw("INSERT INTO channel(id,type,display_name) VALUES ('chfixx00000000000000000001','O','initial')").await.unwrap();
    let mut h=Harness::new_with_identity(&[1],"viewer-444","tenant-1","","","");
    h.step(Tick::Inbound(InboundBytes(serde_json::to_vec(&json!({"action":"increment_channel","data":{"id":cid(1),"type":"O","displayName":"old","updateAt":100,"lastEventSeq":0}})).unwrap().into())),1);
    h.sink.clear();h.module.ingest_increment_end(None,&mut h.sink);
    let old=h.sink.as_slice().iter().find_map(|e|if let Effect::PersistAtomic{ops,..}=e {Some(ops.clone())}else{None}).expect("old snapshot batch");
    h.step(Tick::Inbound(InboundBytes(serde_json::to_vec(&json!({"action":"update_channel","data":{"channel":{"id":cid(1),"displayName":"new","updateAt":200}}})).unwrap().into())),2);
    let mut applied=false;
    for effect in h.sink.as_slice() { let result=match effect {Effect::PersistFire{ops}|Effect::Persist{ops,..}=>execute_storage_ops(&db,ops).await,Effect::PersistAtomic{ops,..}=>execute_storage_ops_atomic(&db,ops).await,_=>continue};assert!(matches!(result,PortOutcome::Ok(_)));applied=true; }
    assert!(applied,"online update must produce writes");
    let key=GetSpec{table:"channel",key_col:"id",key_val:SqlValue::Text(cid(1))};
    let before=db.get(key.clone()).await.unwrap().unwrap();
    assert!(before.iter().any(|(k,v)|k=="display_name" && matches!(v,SqlValue::Text(s) if s=="new")),"new online name must really be stored");
    assert!(matches!(execute_storage_ops_atomic(&db,&old).await,PortOutcome::Ok(_)));
    let after=db.get(key).await.unwrap().unwrap();
    assert!(after.iter().any(|(k,v)|k=="display_name" && matches!(v,SqlValue::Text(s) if s=="new")),"old snapshot overwrote newer name: {after:?}");
}
