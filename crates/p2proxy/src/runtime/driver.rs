use color_eyre::Result;
use proxy_core::ports::Actor;
use tokio::sync::mpsc::Receiver;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use super::context::Context;
use super::discovery::DiscoveryEvent;
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
                // Cancel a long-running handle (e.g. discovery mid-connect with
                // its dial timeouts and retry backoff) the moment shutdown
                // fires, so quitting doesn't wait for it to finish.
                tokio::select! {
                    _ = shutdown.cancelled() => return,
                    _ = actor.handle(&ctx, input) => {}
                }
            }
        }
    }
}

/// Spawns the actor set onto the task set, each actor in its own task so a
/// long-running `handle` never stalls another.
///
/// Two driver kinds: channel-driven actors go through the generic [`drive`] and
/// so the slot is generic over the [`Actor`] trait — drop in any actor whose
/// input matches the inbox (an alternative strategy, a test double). The
/// swarm-owning [`NetworkActor`] is the exception: it owns and polls the libp2p
/// `Swarm`, so it needs the bespoke `drive_network` loop rather than the generic
/// channel→handle pattern, and stays concrete.
pub struct Runtime;

impl Runtime {
    pub fn spawn<D>(
        ctx: Context,
        network: NetworkActor,
        network_inbox: Receiver<NetworkCommand>,
        discovery: D,
        discovery_inbox: Receiver<DiscoveryEvent>,
        shutdown: CancellationToken,
        tasks: &mut JoinSet<Result<()>>,
    ) where
        D: Actor<Input = DiscoveryEvent, Output = (), Context = Context> + Send + 'static,
    {
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
