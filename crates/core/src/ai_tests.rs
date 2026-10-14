use super::*;

#[test]
fn provider_for_recognizes_supported_backends() {
    let mut config = LlmConfig {
        provider: "openai".into(),
        ..Default::default()
    };
    assert_eq!(provider_for(&config).name(), "openai");
    config.provider = "ollama".into();
    assert_eq!(provider_for(&config).name(), "ollama");
    config.provider = "unknown".into();
    assert_eq!(provider_for(&config).name(), "mock");
}

#[test]
fn endpoint_defaults_per_provider() {
    let mut config = LlmConfig {
        provider: "openai".into(),
        model: None,
        ..Default::default()
    };
    let (url, model) = endpoint_for(&config);
    assert_eq!(url, "https://api.openai.com/v1/chat/completions");
    assert_eq!(model, "gpt-4o-mini");

    config.provider = "ollama".into();
    let (url, model) = endpoint_for(&config);
    assert_eq!(url, "http://localhost:11434/v1/chat/completions");
    assert_eq!(model, "llama3");

    config.base_url = Some("http://localhost:8080/v1".into());
    config.model = Some("qwen:7b".into());
    assert_eq!(
        endpoint_for(&config).0,
        "http://localhost:8080/v1/chat/completions"
    );
}

#[test]
fn role_strings_match_openai_contract() {
    assert_eq!(role_str(&Role::System), "system");
    assert_eq!(role_str(&Role::User), "user");
    assert_eq!(role_str(&Role::Assistant), "assistant");
}

// The pool is built once on the first request and keeps working across
// many: this submits more jobs than the pool is wide and expects every
// result back. If the hand-off lock were held across the job, jobs past
// the pool size would still complete - what it would not do is finish
// them in parallel; liveness is what this pins against a deadlock.
#[test]
fn chat_pool_completes_more_jobs_than_it_has_workers() {
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let submitted = (CHAT_POOL_SIZE * 2) as i32;
    for i in 0..submitted {
        let tx = done_tx.clone();
        chat_jobs()
            .send(Box::new(move || {
                let _ = tx.send(i);
            }))
            .unwrap();
    }
    drop(done_tx);
    let mut done: Vec<i32> = done_rx.iter().collect();
    done.sort_unstable();
    let expected: Vec<i32> = (0..submitted).collect();
    assert_eq!(done, expected, "every submitted job must run");
}
