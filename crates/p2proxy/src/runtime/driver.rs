use color_eyre::Result;
use proxy_core::ports::Actor;
use tokio::sync::mpsc::Receiver;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use super::context::Context;
use super::discovery::{DiscoveryActor, DiscoveryEvent};
use super::network::{NetworkActor, NetworkCommand, drive_network};

/// Generic driver for a message-only actor: pull each input from the inbox and
/// dispatch it to `handle` with the shared context. The dispatch lives here, in
/// the runtime — the actor never loops or calls itself.
pub async fn drive<A>(
    mut actor: A,
    mut inbox: Receiver<A::Input>,
    ctx: Context,
    shutdown: CancellationToken,
) where
    A: Actor<Context = Context> + Send,
    A::Input: Send,
{
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return,
            maybe = inbox.recv() => {
                let Some(input) = maybe else { return };
                let _ = actor.handle(&ctx, input).await;
            }
        }
    }
}

/// Spawns every actor onto the task set, each in its own task so a long-running
/// `handle` (e.g. a discovery retry loop) never stalls the swarm driver. The
/// swarm-owning network actor uses its specialised driver; message-only actors
/// use the generic one.
pub struct Runtime;

impl Runtime {
    pub fn spawn(
        ctx: Context,
        network: NetworkActor,
        network_inbox: Receiver<NetworkCommand>,
        discovery: DiscoveryActor,
        discovery_inbox: Receiver<DiscoveryEvent>,
        shutdown: CancellationToken,
        tasks: &mut JoinSet<Result<()>>,
    ) {
        let net_ctx = ctx.clone();
        let net_shutdown = shutdown.clone();
        tasks.spawn(async move {
            drive_network(network, network_inbox, net_ctx, net_shutdown).await;
            Ok(())
        });
        tasks.spawn(async move {
            drive(discovery, discovery_inbox, ctx, shutdown).await;
            Ok(())
        });
    }
}
