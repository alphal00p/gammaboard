use anyhow::Result;
use std::sync::OnceLock;

static SYMBOLICA_OEM_ACTIVATION: OnceLock<Result<(), String>> = OnceLock::new();

pub fn activate_symbolica_oem_license() -> Result<()> {
    let result = SYMBOLICA_OEM_ACTIVATION.get_or_init(|| {
        if option_env!("NO_SYMBOLICA_OEM_LICENSE").is_some() {
            return Ok(());
        }

        std::panic::catch_unwind(|| {
            symbolica::activate_oem_license!("SYMBOLICA_OEM_KEY_7facf394");
        })
        .map_err(|panic| {
            panic
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| panic.downcast_ref::<&str>().copied())
                .unwrap_or("unknown Symbolica OEM activation panic")
                .to_owned()
        })
    });

    result.clone().map_err(|message| {
        anyhow::anyhow!(
            "failed to activate the bundled GammaBoard Symbolica OEM license: {message}. \
             To use a regular Symbolica license instead, rebuild with \
             NO_SYMBOLICA_OEM_LICENSE=1 and set SYMBOLICA_LICENSE at runtime."
        )
    })
}
