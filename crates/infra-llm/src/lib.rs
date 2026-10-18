//! Adaptador de infraestructura para proveedores de LLM.
use harness_ports::llm::LlmClient;

pub struct DefaultLlmClient;

impl LlmClient for DefaultLlmClient {}
