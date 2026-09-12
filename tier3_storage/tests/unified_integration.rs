use tier3_storage::EngineNode;
use tempfile::tempdir;
use std::sync::{Arc, Mutex};
use std::thread;

#[test]
fn test_unified_system_flow() {
    let dir = tempdir().unwrap();
    let engine = Arc::new(Mutex::new(EngineNode::open(dir.path()).unwrap()));

    let mut writers = Vec::new();
    let thread_count = 4;
    let ops_per_thread = 5000;

    // Simulate pinned Tier 1 network threads executing put requests
    for t in 0..thread_count {
        let engine_clone = engine.clone();
        
        let handle = thread::spawn(move || {
            // Register this thread with the engine's EBR collector ONCE at startup
            let thread_idx = {
                let node = engine_clone.lock().unwrap();
                node.collector().register()
            };

            for i in 0..ops_per_thread {
                let key = format!("thread-{}-k-{}", t, i).into_bytes();
                let val = format!("val-{}", i).into_bytes();
                let seq = (t * ops_per_thread + i) as u64;
                
                let mut node = engine_clone.lock().unwrap();
                node.put(&key, &val, seq, thread_idx).unwrap();
            }

            // Cleanly unregister thread on termination
            {
                let node = engine_clone.lock().unwrap();
                node.collector().unregister(thread_idx);
            }
        });
        writers.push(handle);
    }

    for t in writers {
        t.join().unwrap();
    }

    // Read back data under EBR memory protection
    let node = engine.lock().unwrap();
    
    // Register temporary thread index for the main test verification thread
    let main_thread_idx = node.collector().register();

    for t in 0..thread_count {
        for i in 0..ops_per_thread {
            let key = format!("thread-{}-k-{}", t, i).into_bytes();
            let expected_val = format!("val-{}", i).into_bytes();
            
            let lookup = node.get(&key, main_thread_idx);
            assert!(lookup.is_some(), "Key not found: {:?}", String::from_utf8(key));
            let (val, _seq) = lookup.unwrap();
            assert_eq!(val, Some(expected_val));
        }
    }

    node.collector().unregister(main_thread_idx);
}