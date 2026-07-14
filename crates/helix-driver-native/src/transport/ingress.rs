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
    pub(super) async fn send(&self, tick: Tick) -> Result<(), Tick> {
        match self {
            Self::Raw(tx) => tx.send(tick).await.map_err(|error| error.0),
            Self::Stamped(tx) => tx.send(tick).await,
        }
    }

    pub(super) fn attach(&self, client: SharedWsClient, id: TransportId) -> SharedWsClient {
        match self {
            Self::Raw(tx) => client.with_inbound_tick(id, tx.clone()),
            Self::Stamped(tx) => client.with_stamped_inbound_tick(id, tx.clone()),
        }
    }
}
