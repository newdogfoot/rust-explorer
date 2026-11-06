use rust_explorer::core::{
    listing::{ScanUpdate, scan_directory, search_directory},
    model::{Entry, SortField, search_key},
    worker::Cancellation,
};
use serde_json::json;
use std::{path::PathBuf, time::Instant};

fn p95(values: &[f64]) -> f64 {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    v[((v.len() as f64 * 0.95).ceil() as usize).saturating_sub(1)]
}
fn main() {
    let dirs: Vec<_> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if dirs.is_empty() {
        eprintln!("usage: explorer-bench FOLDER [FOLDER ...]");
        std::process::exit(2);
    }
    let mut results = vec![];
    for path in dirs {
        let mut first = vec![];
        let mut total = vec![];
        let mut filters = vec![];
        let mut count = 0;
        for _ in 0..20 {
            let start = Instant::now();
            let mut first_time = None;
            let mut entries: Vec<Entry> = vec![];
            scan_directory(
                &path,
                false,
                SortField::Name,
                false,
                &Cancellation::default(),
                |event| match event {
                    ScanUpdate::Batch(_) => {
                        first_time.get_or_insert_with(|| start.elapsed().as_secs_f64() * 1000.);
                    }
                    ScanUpdate::Complete { entries: e, .. } => entries = e,
                    ScanUpdate::Failed(e) => panic!("{e}"),
                },
            );
            total.push(start.elapsed().as_secs_f64() * 1000.);
            first.push(first_time.unwrap_or(*total.last().unwrap()));
            count = entries.len();
            let start = Instant::now();
            let key = search_key("자료_");
            let found = entries.iter().filter(|e| e.key.contains(&key)).count();
            std::hint::black_box(found);
            filters.push(start.elapsed().as_secs_f64() * 1000.);
        }
        let start = Instant::now();
        let mut found = 0;
        search_directory(
            &path,
            "자료_000",
            false,
            &Cancellation::default(),
            |event| {
                if let ScanUpdate::Complete { entries, .. } = event {
                    found = entries.len()
                }
            },
        );
        let search_ms = start.elapsed().as_secs_f64() * 1000.;
        results.push(json!({"folder":path,"entries":count,"iterations":20,"first_batch_p95_ms":p95(&first),"full_scan_and_sort_p95_ms":p95(&total),"filter_p95_ms":p95(&filters),"recursive_search_ms":search_ms,"search_matches":found,"cache":"OS cache retained; fresh application scan each iteration"}));
    }
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    let rss = unsafe {
        libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr());
        usage.assume_init().ru_maxrss
    };
    println!("{}",serde_json::to_string_pretty(&json!({"benchmarks":results,"peak_rss_mib":rss as f64/(1024.*1024.),"target":std::env::consts::ARCH})).unwrap());
}
