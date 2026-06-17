use std::future::Future;

/// A passive, message-driven actor. It never loops or spawns itself — a runtime
/// owns it, pulls its typed `Input`s, and dispatches each to `handle` with the
/// shared `Context`. Each actor declares its own `Input`/`Output`/`Error`, so it
/// owns its vocabulary, and is unit-testable by calling `handle` directly.
pub trait Actor {
    type Input;
    type Output;
    type Error;
    type Context;

    fn handle(
        &mut self,
        ctx: &Self::Context,
        event: Self::Input,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send;
}
