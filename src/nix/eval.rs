use serde::Deserialize;

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalOutput {
    pub attr: String,
    pub attr_path: Vec<String>,
    pub drv_path: String,
    pub name: String,
    pub required_system_features: Vec<String>,
    pub system: String,
}
