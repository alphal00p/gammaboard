use anyhow::Result;
use std::sync::OnceLock;

static SYMBOLICA_OEM_ACTIVATION: OnceLock<Result<(), String>> = OnceLock::new();

pub fn activate_symbolica_oem_license() -> Result<()> {
    let result = SYMBOLICA_OEM_ACTIVATION.get_or_init(|| {
        use symbolica::license::LicenseManager;

        if option_env!("NO_SYMBOLICA_OEM_LICENSE").is_none() {
            return LicenseManager::set_application_key(
                "SO-422-gammaboard-2028.01.01-WNE74AQ3JKIKBGPTELBWRVQBQX3WG7I6UXPDBE6CT5QYMPDDKA5TI57D2RST5D7NIKSNILKFUA7KGE7YBCFWVB3MD2FHRZQYQ4K76AA",
                env!("CARGO_CRATE_NAME"),
            );
        }
        if LicenseManager::is_licensed() {
            Ok(())
        } else {
            Err("no valid Symbolica 3 license is configured".to_owned())
        }
    });

    result.clone().map_err(|message| {
        anyhow::anyhow!(
            "failed to activate Symbolica: {message}. To use a regular Symbolica v3 license, \
             rebuild with NO_SYMBOLICA_OEM_LICENSE=1 and set SYMBOLICA_LICENSE at runtime."
        )
    })
}
