#[path = "support/p2_common.rs"]
mod p2_common;
#[path = "support/p2_content.rs"]
mod p2_content;
#[path = "support/p2_ignore_content.rs"]
mod p2_ignore_content;
#[path = "support/p2_traversal.rs"]
mod p2_traversal;
#[path = "support/p2_watch.rs"]
mod p2_watch;
mod support;

use p2_content::{benchmark_content_visit, benchmark_multi_content_visit};
use p2_traversal::{
    benchmark_directory_callbacks, benchmark_parallel_multi_stream, benchmark_parallel_pull,
    benchmark_root_policy,
};
use p2_watch::benchmark_watcher_adapter;
use support::{Fixture, SOURCE_FILES};

fn main() {
    let fixture = Fixture::new();
    let second_fixture = Fixture::new();

    println!(
        "corpus=synthetic source_files={SOURCE_FILES} statistic=median runs=11 warmups=2 suite=p2"
    );
    benchmark_parallel_pull(&fixture);
    benchmark_directory_callbacks(&fixture);
    benchmark_parallel_multi_stream(&fixture);
    benchmark_content_visit(&fixture);
    benchmark_multi_content_visit(&fixture, &second_fixture);
    benchmark_root_policy(&fixture);
    benchmark_watcher_adapter(&fixture);
}
