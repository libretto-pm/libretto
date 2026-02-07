//! Composer-compatible platform package detection.
//!
//! Mirrors Composer's `PlatformRepository::isPlatformPackage` semantics:
//! <https://github.com/composer/composer/blob/main/src/Composer/Repository/PlatformRepository.php>

/// Returns `true` when `name` is a Composer platform package.
///
/// Supported names:
/// - `php`, `php-64bit`, `php-ipv6`, `php-zts`, `php-debug`
/// - `hhvm`
/// - `ext-*`, `lib-*` with Composer's character rules
/// - `composer`, `composer-plugin-api`, `composer-runtime-api`
#[must_use]
pub fn is_platform_package_name(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase();
    let n = normalized.as_str();

    if matches!(
        n,
        "php"
            | "php-64bit"
            | "php-ipv6"
            | "php-zts"
            | "php-debug"
            | "hhvm"
            | "composer"
            | "composer-plugin-api"
            | "composer-runtime-api"
    ) {
        return true;
    }

    if let Some(rest) = n.strip_prefix("ext-").or_else(|| n.strip_prefix("lib-")) {
        return is_platform_feature_name(rest);
    }

    false
}

#[must_use]
fn is_platform_feature_name(rest: &str) -> bool {
    // Composer regex equivalent:
    // [a-z0-9](?:[_.-]?[a-z0-9]+)*
    let mut chars = rest.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }

    let mut prev_was_separator = false;
    for c in chars {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            prev_was_separator = false;
            continue;
        }

        if matches!(c, '.' | '_' | '-') && !prev_was_separator {
            prev_was_separator = true;
            continue;
        }

        return false;
    }

    !prev_was_separator
}

#[cfg(test)]
mod tests {
    use super::is_platform_package_name;

    #[test]
    fn composer_compatible_platform_packages() {
        assert!(is_platform_package_name("php"));
        assert!(is_platform_package_name("php-64bit"));
        assert!(is_platform_package_name("php-ipv6"));
        assert!(is_platform_package_name("php-zts"));
        assert!(is_platform_package_name("php-debug"));
        assert!(is_platform_package_name("hhvm"));
        assert!(is_platform_package_name("ext-json"));
        assert!(is_platform_package_name("EXT-mbstring"));
        assert!(is_platform_package_name("lib-openssl"));
        assert!(is_platform_package_name("lib-icu-uc"));
        assert!(is_platform_package_name("composer"));
        assert!(is_platform_package_name("composer-plugin-api"));
        assert!(is_platform_package_name("composer-runtime-api"));
    }

    #[test]
    fn non_platform_packages() {
        assert!(!is_platform_package_name("php-open-source-saver/jwt-auth"));
        assert!(!is_platform_package_name("php-http/discovery"));
        assert!(!is_platform_package_name("symfony/console"));
        assert!(!is_platform_package_name("ext-"));
        assert!(!is_platform_package_name("lib-"));
        assert!(!is_platform_package_name("ext--foo"));
        assert!(!is_platform_package_name("lib.foo"));
    }
}
