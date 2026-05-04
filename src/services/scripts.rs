pub fn run_script(name: &str) -> String {
    match name {
        "test" => "Test executed".into(),
        _ => "Unknown command".into(),
    }
}