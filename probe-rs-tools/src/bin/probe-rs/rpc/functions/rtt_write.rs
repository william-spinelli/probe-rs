use crate::rpc::{
    Key,
    functions::{NoResponse, RpcContext},
};
use postcard_rpc::header::VarHeader;
use postcard_schema::Schema;
use probe_rs::Session;
use serde::{Deserialize, Serialize};

use crate::util::rtt::client::RttClient;

#[derive(Serialize, Deserialize, Schema)]
pub struct WriteRttDownChannelRequest {
    pub sessid: Key<Session>,
    pub rtt_client: Key<RttClient>,
    pub channel: u32,
    pub data: Vec<u8>,
}

pub async fn write_rtt_down_channel(
    ctx: &mut RpcContext,
    _header: VarHeader,
    request: WriteRttDownChannelRequest,
) -> NoResponse {
    let mut session = ctx.session(request.sessid).await;
    let mut rtt_client = ctx.object_mut(request.rtt_client).await;

    let mut core = session.core(rtt_client.core_id())?;
    rtt_client.write_down_channel(&mut core, request.channel, &request.data)?;

    Ok(())
}
