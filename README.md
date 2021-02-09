# waypoints

## Summary

This is a helper crate for writing unit tests and debugging in Rust.  It allows
specifying a sequence at which points of execution may be passed in
multi-threaded code.  Execution will not pass a waypoint until its designated
number is reached.

When writing unit tests (where brevity and simplicity wins over style) it can
be helpful to enforce an execution order on points in different threads to
assess whether a particular condition is handled appropriately.  This crate is
a way to do that.  While `waypoints` could be used outside unit tests or
debugging, there are likely more robust solutions.

The docs can be found on [here][docs_url].

## Example

The example below uses two threads to push numbers 0-5 onto a vector;
`waypoints` are set to enforce the order in which the numbers are pushed.

```rust
use waypoints::Waypoints;
use std::sync::{Arc, Mutex};

// A vector of observations
let obs = Arc::new(Mutex::new(Vec::new()));

// A thread-safe series of waypoints
let w = Waypoints::new_arc();

let mut threads = Vec::new();
threads.push({
    let obs = obs.clone();
    let w = w.clone();
    std::thread::spawn(move || {
        obs.lock().unwrap().push(0);
        obs.lock().unwrap().push(1);
        // the other thread may not proceed past waypoint 1
        // until waypoint 0 is passed
        w.point(0, None).unwrap();
        // this thread has to wait on waypoint 2 to be
        // passed in the other thread before proceeding
        w.point(3, None).unwrap();
        obs.lock().unwrap().push(4);
        obs.lock().unwrap().push(5);
    })
});

threads.push({
    let obs = obs.clone();
    let w = w.clone();
    std::thread::spawn(move || {
        w.point(1, None).unwrap();
        obs.lock().unwrap().push(2);
        obs.lock().unwrap().push(3);
        w.point(2, None).unwrap();
    })
});

threads.into_iter().for_each(|t| {
    t.join().ok();
});

let obs = Arc::try_unwrap(obs).unwrap().into_inner().unwrap();
println!("obs: {:?}", &obs); // obs: [0, 1, 2, 3, 4, 5]
assert_eq!(obs, (0..6).into_iter().collect::<Vec<_>>());
```
[docs_url]: https://trtsl.github.io/waypoints/waypoints/index.html
