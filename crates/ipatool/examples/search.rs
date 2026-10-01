//! Search the public iTunes Search API (no Apple ID required).
//!
//! ```bash
//! cargo run -p ipatool-kit --example search -- "vk"
//! ```

use ipatool_kit::{AppStoreClient, HttpClient};

fn main() {
    let query = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "calculator".into());
    let country = std::env::var("IPATOOL_COUNTRY").unwrap_or_else(|_| "us".into());
    let client = HttpClient::new().with_country(&country);

    match client.search(&query, 5, None) {
        Ok(results) => {
            for app in results.results {
                println!("{:>12}  {}  ({})", app.id, app.name, app.bundle_id);
            }
        }
        Err(e) => {
            eprintln!("search failed: {e}");
            std::process::exit(1);
        }
    }
}
