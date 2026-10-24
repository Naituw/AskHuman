//! Retain Nix-declared GSettings data paths when launching a copied ELF directly.

pub(crate) fn configure_gsettings() {
    let compiled = env!("ASKHUMAN_COMPILED_GSETTINGS_DATA_DIRS");
    let existing = std::env::var_os("XDG_DATA_DIRS");
    if let Some(merged) = crate::gsettings_schema::merged_data_dirs(compiled, existing.as_deref()) {
        // Called from main before application threads, GTK, or Dev re-exec start.
        std::env::set_var("XDG_DATA_DIRS", merged);
    }
}
