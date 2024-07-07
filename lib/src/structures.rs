use serde::Deserialize;

#[derive(Deserialize, Debug, Clone, Default)]
pub struct CrateData {
    pub name: String,
    pub vers: String,
    pub cksum: String,
    pub yanked: bool
}

#[derive(Deserialize, Debug, Default)]
pub struct RepoConfig {
    pub dl: String,
}
