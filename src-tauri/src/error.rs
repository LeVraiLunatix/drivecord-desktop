use serde::{Serialize, Serializer};

/// Everything a command can fail with. Serialised to a plain string when it
/// crosses the IPC boundary, so the frontend just gets a readable message.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("clé API invalide ou manquante")]
    Unauthorized,

    #[error("la clé API n'a pas la permission « {0} »")]
    MissingScope(&'static str),

    #[error("ressource introuvable")]
    NotFound,

    #[error("trop de requêtes (429) — réessai automatique épuisé")]
    RateLimited,

    #[error("réponse inattendue du serveur ({status}) : {body}")]
    Unexpected { status: u16, body: String },

    #[error("erreur réseau : {0}")]
    Network(String),

    #[error("stockage sécurisé indisponible : {0}")]
    Keyring(String),

    #[error("configuration : {0}")]
    Config(String),

    #[error("{0}")]
    Other(String),
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl From<reqwest::Error> for AppError {
    fn from(e: reqwest::Error) -> Self {
        AppError::Network(e.to_string())
    }
}

impl From<keyring::Error> for AppError {
    fn from(e: keyring::Error) -> Self {
        AppError::Keyring(e.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
