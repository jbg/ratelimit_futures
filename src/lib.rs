//! Rate-limiting for futures.
//!
//! This crate hooks the
//! [ratelimit_meter](https://crates.io/crates/ratelimit_meter) crate up
//! to futures v0.1 (the same version supported by Tokio right now).
//!
//! # Usage & mechanics of rate limiting with futures
//!
//! To use this crate's Future type, use the provided `Ratelimit::new`
//! function. It takes a direct rate limiter (an in-memory rate limiter
//! implementation), and returns a Future that can be chained to the
//! actual work that you mean to perform:
//!
//! ```rust
//! use std::num::NonZeroU32;
//!
//! use ratelimit_meter::{DirectRateLimiter, LeakyBucket};
//! use ratelimit_futures::Ratelimit;
//!
//!
//! let mut lim = DirectRateLimiter::<LeakyBucket>::per_second(NonZeroU32::new(1).unwrap());
//! {
//!     let mut lim = lim.clone();
//!     Ratelimit::new(&mut lim).await?;
//! }
//! {
//!     let mut lim = lim.clone();
//!     Ratelimit::new(&mut lim).await?;
//! }
//! ```
//!
//! In this example, we're constructing futures that can each start work
//! only once the (shared) rate limiter says that it's ok to start.
//!
//! You can probably guess the mechanics of using these rate-limiting
//! futures:
//!
//! * Construct and a single rate limiter for the work that needs to count
//!   against that rate limit. You can share them using their `Clone`
//!   trait.
//! * Rate-limiting futures will wait as long as it takes to arrive at a
//!   point where code is allowed to proceed. If the shared rate limiter
//!   already allowed another piece of code to proceed, the wait time will
//!   be extended.


use std::{
    future::Future,
    pin::Pin,
    task::Poll,
    time::Instant,
};

use futures_timer::Delay;
use ratelimit_meter::{
    algorithms::Algorithm,
    DirectRateLimiter,
    NonConformance,
};


/// The rate-limiter as a future.
pub struct Ratelimit<'a, A: Algorithm<Instant>>
where
    <A as Algorithm>::NegativeDecision: NonConformance,
{
    delay: Pin<Box<Delay>>,
    limiter: &'a mut DirectRateLimiter<A>,
    first_time: bool,
}

impl<'a, A: Algorithm<Instant>> Ratelimit<'a, A>
where
    <A as Algorithm>::NegativeDecision: NonConformance,
{
    /// Check if the rate-limiter would allow a request through.
    fn check(&mut self) -> Result<(), ()> {
        match self.limiter.check() {
            Ok(()) => Ok(()),
            Err(nc) => {
                let earliest = nc.earliest_possible();
                self.delay.reset(earliest);
                Err(())
            }
        }
    }

    /// Creates a new future that resolves successfully as soon as the
    /// rate limiter allows it.
    pub fn new(limiter: &'a mut DirectRateLimiter<A>) -> Self {
        Ratelimit {
            delay: Box::pin(Delay::new(Default::default())),
            first_time: true,
            limiter,
        }
    }
}

impl<'a, A: Algorithm<Instant>> Future for Ratelimit<'a, A>
where
    <A as Algorithm>::NegativeDecision: NonConformance,
{
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        if self.first_time {
            // First time we run, let's check the rate-limiter and set
            // up a delay if we can't proceed:
            self.first_time = false;
            if self.check().is_ok() {
                return Poll::Ready(());
            }
        }
        match self.delay.as_mut().poll(cx) {
            // Timer says we should check the rate-limiter again, do
            // it and reset the delay otherwise.
            Poll::Ready(_) => match self.check() {
                Ok(_) => Poll::Ready(()),
                Err(_) => Poll::Pending,
            },

            // timer isn't yet ready, let's wait:
            Poll::Pending => Poll::Pending,
        }
    }
}
