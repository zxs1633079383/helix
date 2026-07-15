use helix_core::effect::TransportId;
use helix_core::Tick;
use helix_driver_host::{SharedWsClient, TickIngressSender};
use tokio::sync::mpsc;

#[derive(Clone)]
pub(super) enum NativeTickSender {
    Raw(mpsc::Sender<Tick>),
    Stamped(TickIngressSender),
}

impl NativeTickSender {
    pub(super) fn attach(&self, client: SharedWsClient, id: TransportId) -> SharedWsClient {
        match self {
            Self::Raw(tx) => client.with_inbound_tick(id, tx.clone()),
            Self::Stamped(tx) => client.with_stamped_inbound_tick(id, tx.clone()),
        }
    }
}
