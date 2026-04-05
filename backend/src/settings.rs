use serde::{Deserialize, Serialize};
use shared::extensions::settings::{
    ExtensionSettings, SettingsDeserializeExt, SettingsDeserializer, SettingsSerializeExt,
    SettingsSerializer,
};
#[derive(Serialize, Deserialize, Clone)]
pub struct ContentInstallerSettings {
    pub curseforge_api_key: compact_str::CompactString,
}

impl Default for ContentInstallerSettings {
    fn default() -> Self {
        Self {
            curseforge_api_key: "".into(),
        }
    }
}

#[async_trait::async_trait]
impl SettingsSerializeExt for ContentInstallerSettings {
    async fn serialize(
        &self,
        serializer: SettingsSerializer,
    ) -> Result<SettingsSerializer, anyhow::Error> {
        Ok(serializer.write_raw_setting("curseforge_api_key", self.curseforge_api_key.clone()))
    }
}

pub struct ContentInstallerSettingsDeserializer;

#[async_trait::async_trait]
impl SettingsDeserializeExt for ContentInstallerSettingsDeserializer {
    async fn deserialize_boxed(
        &self,
        mut deserializer: SettingsDeserializer<'_>,
    ) -> Result<ExtensionSettings, anyhow::Error> {
        let defaults = ContentInstallerSettings::default();

        let curseforge_api_key = deserializer
            .take_raw_setting("curseforge_api_key")
            .unwrap_or(defaults.curseforge_api_key);

        Ok(Box::new(ContentInstallerSettings {
            curseforge_api_key,
        }))
    }
}
