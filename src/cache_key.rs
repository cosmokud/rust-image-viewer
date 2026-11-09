//! Single owner of cache-key identity: path -> key string.
//!
//! Resolution (absolute kept as-is, relative canonicalized), lossy `String`, Windows case folding,
//! empty rejection and suffix spelling all live here.
//!
//! Only the *suffix* may differ between caches. `KeyScope::Bare` must stay byte-identical to what
//! redb already persists on users' disks (metadata + folder-travel tables) -> no prefix stripping,
//! no separator normalization, no added folding.

use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyScope {
    Bare,
    /// Clamped to >=1; in-memory only, but `#ts{side}` spelling is embedded in texture ids.
    TextureSide(u32),
    /// `layout` is the caller's layout name, emitted verbatim after `#`.
    TravelLayout(&'static str),
}

pub fn key(path: &Path, scope: KeyScope) -> Option<String> {
    let normalized = if path.is_absolute() {
        path.to_path_buf()
    } else {
        path.canonicalize()
            .ok()
            .unwrap_or_else(|| path.to_path_buf())
    };

    let bare = normalized.to_string_lossy().to_string();
    if bare.is_empty() {
        return None;
    }

    #[cfg(target_os = "windows")]
    let bare = bare.to_lowercase();

    Some(match scope {
        KeyScope::Bare => bare,
        KeyScope::TextureSide(side) => format!("{}#ts{}", bare, side.max(1)),
        KeyScope::TravelLayout(layout) => format!("{}#{}", bare, layout),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Absent dir -> no canonicalize, so the key is the spelling itself; no temp/link privileges.
    fn absent_path() -> std::path::PathBuf {
        std::env::temp_dir()
            .join("riv-cache-key-absent")
            .join("p.jpg")
    }

    #[test]
    fn scope_changes_key_but_same_scope_is_stable() {
        let path = absent_path();
        let bare = key(&path, KeyScope::Bare).unwrap();
        let side = key(&path, KeyScope::TextureSide(512)).unwrap();
        let travel = key(&path, KeyScope::TravelLayout("masonry")).unwrap();

        assert_ne!(bare, side);
        assert_ne!(bare, travel);
        assert_ne!(side, travel);
        assert_eq!(bare, key(&path, KeyScope::Bare).unwrap());
        assert_eq!(side, key(&path, KeyScope::TextureSide(512)).unwrap());
        assert_eq!(
            travel,
            key(&path, KeyScope::TravelLayout("masonry")).unwrap()
        );
    }

    // shortcut: Windows keys from absolute paths skip canonicalize, so they lack the `\\?\`
    // verbatim prefix that relative keys carry. Upgrade only with a schema-compat decision.
    #[test]
    fn relative_and_absolute_spellings_converge() {
        let rel = Path::new("Cargo.toml");
        let abs = std::env::current_dir().unwrap().join(rel);
        let rel_key = key(rel, KeyScope::Bare).unwrap();
        let abs_key = key(&abs, KeyScope::Bare).unwrap();

        #[cfg(not(target_os = "windows"))]
        assert_eq!(rel_key, abs_key);

        #[cfg(target_os = "windows")]
        assert_eq!(rel_key.trim_start_matches(r"\\?\"), abs_key);
    }

    #[test]
    fn case_folding_follows_platform() {
        let mixed = std::env::temp_dir()
            .join("RIV-Cache-Key-CaSe")
            .join("Photo.JPG");
        let lower = Path::new(&mixed.to_string_lossy().to_lowercase()).to_path_buf();

        let mixed_key = key(&mixed, KeyScope::Bare).unwrap();
        let lower_key = key(&lower, KeyScope::Bare).unwrap();

        #[cfg(target_os = "windows")]
        assert_eq!(mixed_key, lower_key);

        #[cfg(not(target_os = "windows"))]
        assert_ne!(mixed_key, lower_key);
    }

    #[test]
    fn texture_side_suffix_clamps_zero_and_one() {
        let path = absent_path();
        let zero = key(&path, KeyScope::TextureSide(0)).unwrap();
        let one = key(&path, KeyScope::TextureSide(1)).unwrap();

        assert_eq!(zero, one);
        assert!(one.ends_with("#ts1"));
        assert!(key(&path, KeyScope::TextureSide(2048))
            .unwrap()
            .ends_with("#ts2048"));
    }

    #[test]
    fn empty_path_yields_none() {
        assert!(key(Path::new(""), KeyScope::Bare).is_none());
        assert!(key(Path::new(""), KeyScope::TextureSide(64)).is_none());
        assert!(key(Path::new(""), KeyScope::TravelLayout("masonry")).is_none());
    }

    #[test]
    fn bare_has_no_suffix_and_travel_ends_with_layout() {
        let path = absent_path();

        assert!(!key(&path, KeyScope::Bare).unwrap().contains('#'));
        assert!(key(&path, KeyScope::TravelLayout("long_strip"))
            .unwrap()
            .ends_with("#long_strip"));
    }
}
