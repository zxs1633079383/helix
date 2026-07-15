use bytes::Bytes;
use helix_core::ports::FrameSender;
use helix_core::PortError;

struct SendOnly;

#[async_trait::async_trait]
impl FrameSender for SendOnly {
    async fn send(&self, _: Bytes) -> Result<(), PortError> {
        Ok(())
    }
}

fn accept_dyn_sender(_: &dyn FrameSender) {}

#[test]
fn frame_sender_requires_only_send_and_is_dyn_safe() {
    let sender = SendOnly;
    accept_dyn_sender(&sender);
}
