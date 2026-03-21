#[tokio::main]
async fn main() {
    tokio::select! {
        _ = async { () } => {}
    }
}
