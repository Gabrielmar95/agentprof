use serde_json::Value;
use std::sync::OnceLock;
use tiktoken_rs::CoreBPE;

static BPE: OnceLock<CoreBPE> = OnceLock::new();

fn bpe() -> &'static CoreBPE {
    BPE.get_or_init(|| tiktoken_rs::cl100k_base().expect("failed to initialize tokenizer"))
}

pub fn count_tokens(text: &str) -> usize {
    bpe().encode_with_special_tokens(text).len()
}

pub fn count_tokens_json(value: &Value) -> usize {
    let text = serde_json::to_string(value).unwrap_or_default();
    count_tokens(&text)
}
