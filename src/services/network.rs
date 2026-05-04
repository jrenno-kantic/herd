use tokio::time::{sleep, Duration};

pub async fn scan_devices() -> String {
    sleep(Duration::from_millis(500)).await;
    "No devices discovered".into()
}
