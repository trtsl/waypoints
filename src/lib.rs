//! # Summary
//!
//! This is a helper crate for writing unit tests and debugging.  It allows specifying a sequence
//! at which points of execution may be passed in multi-threaded code.  Execution will not pass a
//! waypoint until its designated number is reached.
//!
//! When writing unit tests (where brevity and simplicity wins over style) it can be helpful to
//! enforce an execution order on points in different threads to assess whether a particular
//! condition is handled appropriately.  This crate is a way to do that.  While `waypoints` could
//! be used outside unit tests or debugging, there are likely more robust solutions.
//!
//! The repository can be found on [GitHub][repo_url].
//!
//! # Example
//!
//! The example below uses two threads to push numbers 0-5 onto a vector; `waypoints` are set to
//! enforce the order in which the numbers are pushed.
//!
//! ```
//! use waypoints::Waypoints;
//! use std::sync::{Arc, Mutex};
//!
//! // A vector of observations
//! let obs = Arc::new(Mutex::new(Vec::new()));
//!
//! // A thread-safe series of waypoints
//! let w = Waypoints::new_arc();
//!
//! let mut threads = Vec::new();
//! threads.push({
//!     let obs = obs.clone();
//!     let w = w.clone();
//!     std::thread::spawn(move || {
//!         obs.lock().unwrap().push(0);
//!         obs.lock().unwrap().push(1);
//!         // the other thread may not proceed past waypoint 1
//!         // until waypoint 0 is passed
//!         w.point(0, None).unwrap();
//!         // this thread has to wait on waypoint 2 to be
//!         // passed in the other thread before proceeding
//!         w.point(3, None).unwrap();
//!         obs.lock().unwrap().push(4);
//!         obs.lock().unwrap().push(5);
//!     })
//! });
//!
//! threads.push({
//!     let obs = obs.clone();
//!     let w = w.clone();
//!     std::thread::spawn(move || {
//!         w.point(1, None).unwrap();
//!         obs.lock().unwrap().push(2);
//!         obs.lock().unwrap().push(3);
//!         w.point(2, None).unwrap();
//!     })
//! });
//!
//! threads.into_iter().for_each(|t| {
//!     t.join().ok();
//! });
//!
//! let obs = Arc::try_unwrap(obs).unwrap().into_inner().unwrap();
//! println!("obs: {:?}", &obs); // obs: [0, 1, 2, 3, 4, 5]
//! assert_eq!(obs, (0..6).into_iter().collect::<Vec<_>>());
//! ```
//!
//! [repo_url]: https://github.com/trtsl/waypoints

#![forbid(unsafe_code)]
#![warn(
    rust_2018_idioms,
    missing_debug_implementations,
    missing_docs,
    broken_intra_doc_links
)]

use std::sync::Condvar;
use std::sync::{Arc, LockResult, Mutex, MutexGuard};
use std::time::{Duration, Instant};

type Guard<'a> = MutexGuard<'a, (usize, Option<Instant>)>;

/// Represents a series of waypoints.
///
/// The struct has an internal state containing the next expected waypoint and the earliest time at
/// which it may be passed.  All its methods are accessible via shared references, so typical usage
/// would wrap [`Waypoints`] in an [`Arc`] to make it accessible from different threads.  The
/// function [`Waypoints::new_arc`] creates an `Arc<Waypoints>>`.
#[derive(Debug)]
pub struct Waypoints {
    // tuple element 0: the current waypoint
    // tuple element 1: the earliest time at which the next waypoint may be passed
    state: Mutex<(usize, Option<Instant>)>,
    cv: Condvar,
}

impl Waypoints {
    /// Create `Waypoints`.
    pub fn new() -> Self {
        Self {
            state: Mutex::new((0, None)),
            cv: Condvar::new(),
        }
    }

    /// Create `Waypoints` wrapped in an [`Arc`].
    pub fn new_arc() -> Arc<Self> {
        Arc::new(Self::new())
    }

    fn state_lck(&self) -> Guard<'_> {
        Self::into_guard(self.state.lock())
    }

    fn into_guard(state: LockResult<Guard<'_>>) -> Guard<'_> {
        // the data held by the guard should not be corrupted (none of the operations performed
        // while the lock is held should panic), so `Err` variant should be ok to use
        match state {
            Ok(lck) => lck,
            Err(err) => err.into_inner(),
        }
    }

    /// Reset the `Waypoints` to start at point 0 without an time requirement.
    pub fn reset(&self) {
        self.set(0, None);
    }

    /// Set the `Waypoints` to a particular state.  Argument `t` is the time at which the next
    /// waypoint may pass.
    pub fn set(&self, n: usize, t: Option<Instant>) {
        *self.state_lck() = (n, t);
    }

    /// Allow the waypoint to be passed if the current number matches exactly.  See
    /// [`Self::range`] for the `head_start` argument.
    pub fn point(&self, n: usize, head_start: Option<Duration>) -> Result<(), usize> {
        self.range(n, n, head_start)
    }

    /// Allow a waypoint to be passed if the current number is within the range (inclusive).  This
    /// can be used to have multiple threads pass a waypoint concurrently rather than any
    /// particular thread being advantaged.  Argument `head_start` represents the minimum amount of
    /// time between calling this method and the next waypoint being allowed to pass.  The `Result`
    /// is an `Err` if a another waypoint previously use the same waypoint number.
    pub fn range(&self, l: usize, h: usize, head_start: Option<Duration>) -> Result<(), usize> {
        let state_lck = self.cv.wait_while(self.state_lck(), |&mut (n, _)| n < l);
        let mut state_lck = Self::into_guard(state_lck);

        // check the state
        let res = match *state_lck {
            (n, _) if l <= n && n <= h => Ok(()),
            (n, _) if n > h => Err(n),
            _ => unreachable!("passed waypoint before schedule"),
        };

        // update state
        let (ref mut n, ref mut target_time) = *state_lck;
        *n += 1;
        let now = Instant::now();
        let target_time_this = target_time.clone();
        *target_time = match (*target_time, head_start) {
            (Some(t), Some(dt)) => Some(std::cmp::max(now, t) + dt),
            (Some(t), None) if now < t => Some(t),
            (Some(_), None) => None,
            (None, Some(dt)) => Some(now + dt),
            (None, None) => None,
        };

        // drop lock before sleeping
        drop(state_lck);

        match target_time_this {
            Some(t) if now < t => std::thread::sleep(t - now),
            _ => {}
        }

        self.cv.notify_all();

        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_on_duplicate_waypoint() {
        let w = Waypoints::new();
        w.point(0, None).unwrap();
        assert!(w.point(0, None).is_err());
    }

    #[test]
    fn sequence() {
        let v_point = Arc::new(Mutex::new(Vec::new()));
        let v_range = Arc::new(Mutex::new(Vec::new()));
        let w = Waypoints::new_arc();
        let mut threads = Vec::new();

        threads.push({
            let v_point = v_point.clone();
            let v_range = v_range.clone();
            let w = w.clone();
            std::thread::spawn(move || {
                v_point.lock().unwrap().push(0);
                w.point(0, None).unwrap();
                w.point(3, None).unwrap();
                v_point.lock().unwrap().push(3);
                v_point.lock().unwrap().push(4);
                v_point.lock().unwrap().push(5);
                v_point.lock().unwrap().push(6);
                w.point(4, None).unwrap();
                w.point(7, None).unwrap();
                v_point.lock().unwrap().push(9);
                w.point(8, None).unwrap();
                w.range(9, 11, None).unwrap();
                for _ in 0..3 {
                    std::thread::yield_now();
                    v_range.lock().unwrap().push(11);
                }
            })
        });

        threads.push({
            let v_point = v_point.clone();
            let v_range = v_range.clone();
            let w = w.clone();
            std::thread::spawn(move || {
                w.point(1, None).unwrap();
                v_point.lock().unwrap().push(1);
                v_point.lock().unwrap().push(2);
                w.point(2, None).unwrap();
                w.range(9, 11, None).unwrap();
                for _ in 0..3 {
                    std::thread::yield_now();
                    v_range.lock().unwrap().push(22);
                }
            })
        });

        threads.push({
            let v_point = v_point.clone();
            let v_range = v_range.clone();
            let w = w.clone();
            std::thread::spawn(move || {
                w.point(5, None).unwrap();
                v_point.lock().unwrap().push(7);
                v_point.lock().unwrap().push(8);
                w.point(6, None).unwrap();
                w.range(9, 11, None).unwrap();
                for _ in 0..3 {
                    std::thread::yield_now();
                    v_range.lock().unwrap().push(33);
                }
            })
        });

        threads.into_iter().for_each(|t| {
            t.join().ok();
        });

        let v_point = Arc::try_unwrap(v_point).unwrap().into_inner().unwrap();
        let v_range = Arc::try_unwrap(v_range).unwrap().into_inner().unwrap();
        println!("points are ordered: {:?}", &v_point);
        println!("range can provide concurrency (no order): {:?}", &v_range);
        assert_eq!(v_point, (0..10).into_iter().collect::<Vec<_>>());
    }

    #[test]
    fn head_start() {
        let dt = Duration::from_millis(100);
        let dt_l = dt - Duration::from_millis(10);
        let dt_h = dt + Duration::from_millis(10);
        let mut dt_n = 0;

        let w = Waypoints::new();
        let t0 = Instant::now();

        let assert_msg = "assert failure could be misleading, \
                          due to relying on observed time: run again to confirm";

        w.point(0, Some(dt)).unwrap();
        let dt_observed = t0.elapsed();
        assert!(
            dt_n * dt_l < dt_observed && dt_observed < 1 * dt_h,
            assert_msg
        );

        w.point(1, Some(dt)).unwrap();
        let dt_observed = t0.elapsed();
        dt_n += 1;
        assert!(
            dt_n * dt_l < dt_observed && dt_observed < dt_n * dt_h,
            assert_msg
        );

        std::thread::sleep(dt);

        w.point(2, None).unwrap();
        let dt_observed = t0.elapsed();
        dt_n += 1;
        assert!(
            dt_n * dt_l < dt_observed && dt_observed < dt_n * dt_h,
            assert_msg
        );

        w.point(3, None).unwrap();
        let dt_observed = t0.elapsed();
        assert!(
            dt_n * dt_l < dt_observed && dt_observed < dt_n * dt_h,
            assert_msg
        );

        w.set(6, Some(Instant::now() + dt));
        w.point(6, Some(dt)).unwrap();
        let dt_observed = t0.elapsed();
        dt_n += 1;
        assert!(
            dt_n * dt_l < dt_observed && dt_observed < dt_n * dt_h,
            assert_msg
        );

        std::thread::sleep(5 * dt);

        w.point(7, None).unwrap();
        let dt_observed = t0.elapsed();
        dt_n += 5;
        assert!(
            dt_n * dt_l < dt_observed && dt_observed < dt_n * dt_h,
            assert_msg
        );
    }
}
