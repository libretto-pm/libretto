//! Pre-built test fixtures for common testing scenarios.
//!
//! This module provides static fixtures based on real-world PHP projects
//! like Laravel, Symfony, Drupal, and more.

use serde_json::{Value, json};

/// Collection of pre-built test fixtures.
#[derive(Debug)]
pub struct Fixtures;

impl Fixtures {
    /// Empty composer.json with minimal required fields.
    #[must_use]
    pub fn empty_composer_json() -> Value {
        json!({
            "name": "test/project",
            "description": "Test project",
            "type": "project",
            "require": {},
            "autoload": {}
        })
    }

    /// Simple composer.json with a few dependencies.
    #[must_use]
    pub fn simple_composer_json() -> Value {
        json!({
            "name": "test/simple-project",
            "description": "Simple test project",
            "type": "project",
            "require": {
                "php": ">=8.1",
                "monolog/monolog": "^3.0",
                "guzzlehttp/guzzle": "^7.0"
            },
            "require-dev": {
                "phpunit/phpunit": "^10.0"
            },
            "autoload": {
                "psr-4": {
                    "App\\": "src/"
                }
            }
        })
    }

    /// Laravel-like composer.json with typical dependencies.
    #[must_use]
    pub fn laravel_composer_json() -> Value {
        json!({
            "name": "laravel/laravel",
            "type": "project",
            "description": "The Laravel Framework.",
            "keywords": ["laravel", "framework"],
            "license": "MIT",
            "require": {
                "php": "^8.1",
                "laravel/framework": "^10.10",
                "laravel/sanctum": "^3.2",
                "laravel/tinker": "^2.8",
                "guzzlehttp/guzzle": "^7.2"
            },
            "require-dev": {
                "fakerphp/faker": "^1.9.1",
                "laravel/pint": "^1.0",
                "laravel/sail": "^1.18",
                "mockery/mockery": "^1.4.4",
                "nunomaduro/collision": "^7.0",
                "phpunit/phpunit": "^10.1",
                "spatie/laravel-ignition": "^2.0"
            },
            "autoload": {
                "psr-4": {
                    "App\\": "app/",
                    "Database\\Factories\\": "database/factories/",
                    "Database\\Seeders\\": "database/seeders/"
                }
            },
            "autoload-dev": {
                "psr-4": {
                    "Tests\\": "tests/"
                }
            },
            "scripts": {
                "post-autoload-dump": [
                    "Illuminate\\Foundation\\ComposerScripts::postAutoloadDump",
                    "@php artisan package:discover --ansi"
                ],
                "post-update-cmd": [
                    "@php artisan vendor:publish --tag=laravel-assets --ansi --force"
                ],
                "post-root-package-install": [
                    "@php -r \"file_exists('.env') || copy('.env.example', '.env');\""
                ],
                "post-create-project-cmd": [
                    "@php artisan key:generate --ansi"
                ]
            },
            "extra": {
                "laravel": {
                    "dont-discover": []
                }
            },
            "config": {
                "optimize-autoloader": true,
                "preferred-install": "dist",
                "sort-packages": true,
                "allow-plugins": {
                    "pestphp/pest-plugin": true,
                    "php-http/discovery": true
                }
            },
            "minimum-stability": "stable",
            "prefer-stable": true
        })
    }

    /// Symfony-like composer.json.
    #[must_use]
    pub fn symfony_composer_json() -> Value {
        json!({
            "name": "symfony/symfony-demo",
            "type": "project",
            "description": "Symfony Demo Application",
            "license": "MIT",
            "require": {
                "php": ">=8.1",
                "symfony/console": "^6.3",
                "symfony/framework-bundle": "^6.3",
                "symfony/http-kernel": "^6.3",
                "symfony/routing": "^6.3",
                "symfony/yaml": "^6.3",
                "symfony/twig-bundle": "^6.3",
                "symfony/form": "^6.3",
                "symfony/validator": "^6.3",
                "symfony/security-bundle": "^6.3",
                "doctrine/orm": "^2.15",
                "doctrine/doctrine-bundle": "^2.10",
                "doctrine/doctrine-migrations-bundle": "^3.2"
            },
            "require-dev": {
                "symfony/debug-bundle": "^6.3",
                "symfony/maker-bundle": "^1.50",
                "symfony/phpunit-bridge": "^6.3",
                "symfony/stopwatch": "^6.3",
                "symfony/web-profiler-bundle": "^6.3"
            },
            "autoload": {
                "psr-4": {
                    "App\\": "src/"
                }
            },
            "autoload-dev": {
                "psr-4": {
                    "App\\Tests\\": "tests/"
                }
            },
            "config": {
                "allow-plugins": {
                    "symfony/flex": true,
                    "symfony/runtime": true
                },
                "sort-packages": true
            },
            "extra": {
                "symfony": {
                    "allow-contrib": false,
                    "require": "6.3.*"
                }
            }
        })
    }

    /// Drupal-like composer.json.
    #[must_use]
    pub fn drupal_composer_json() -> Value {
        json!({
            "name": "drupal/recommended-project",
            "type": "project",
            "description": "Drupal recommended project template",
            "license": "GPL-2.0-or-later",
            "require": {
                "php": ">=8.1",
                "composer/installers": "^2.0",
                "drupal/core-composer-scaffold": "^10.0",
                "drupal/core-project-message": "^10.0",
                "drupal/core-recommended": "^10.0",
                "drush/drush": "^12.0"
            },
            "require-dev": {
                "drupal/core-dev": "^10.0"
            },
            "conflict": {
                "drupal/drupal": "*"
            },
            "repositories": [
                {
                    "type": "composer",
                    "url": "https://packages.drupal.org/8"
                }
            ],
            "extra": {
                "drupal-scaffold": {
                    "locations": {
                        "web-root": "web/"
                    }
                },
                "installer-paths": {
                    "web/core": ["type:drupal-core"],
                    "web/libraries/{$name}": ["type:drupal-library"],
                    "web/modules/contrib/{$name}": ["type:drupal-module"],
                    "web/profiles/contrib/{$name}": ["type:drupal-profile"],
                    "web/themes/contrib/{$name}": ["type:drupal-theme"],
                    "drush/Commands/contrib/{$name}": ["type:drupal-drush"]
                }
            },
            "minimum-stability": "stable",
            "prefer-stable": true
        })
    }

    /// `WordPress` Bedrock-like composer.json.
    #[must_use]
    pub fn wordpress_bedrock_composer_json() -> Value {
        json!({
            "name": "roots/bedrock",
            "type": "project",
            "description": "WordPress boilerplate with Composer",
            "license": "MIT",
            "require": {
                "php": ">=8.0",
                "composer/installers": "^2.2",
                "vlucas/phpdotenv": "^5.5",
                "oscarotero/env": "^2.1",
                "roots/bedrock-autoloader": "^1.0",
                "roots/bedrock-disallow-indexing": "^2.0",
                "roots/wordpress": "^6.3",
                "roots/wp-config": "^1.0",
                "roots/wp-password-bcrypt": "^1.1"
            },
            "require-dev": {
                "squizlabs/php_codesniffer": "^3.7",
                "roave/security-advisories": "dev-latest"
            },
            "repositories": [
                {
                    "type": "composer",
                    "url": "https://wpackagist.org",
                    "only": ["wpackagist-plugin/*", "wpackagist-theme/*"]
                }
            ],
            "extra": {
                "installer-paths": {
                    "web/app/mu-plugins/{$name}/": ["type:wordpress-muplugin"],
                    "web/app/plugins/{$name}/": ["type:wordpress-plugin"],
                    "web/app/themes/{$name}/": ["type:wordpress-theme"]
                },
                "wordpress-install-dir": "web/wp"
            }
        })
    }

    /// `PHPUnit` composer.json (testing framework).
    #[must_use]
    pub fn phpunit_composer_json() -> Value {
        json!({
            "name": "phpunit/phpunit",
            "type": "library",
            "description": "The PHP Unit Testing framework.",
            "license": "BSD-3-Clause",
            "require": {
                "php": ">=8.1",
                "ext-dom": "*",
                "ext-json": "*",
                "ext-libxml": "*",
                "ext-mbstring": "*",
                "ext-xml": "*",
                "ext-xmlwriter": "*",
                "myclabs/deep-copy": "^1.10.1",
                "phar-io/manifest": "^2.0.3",
                "phar-io/version": "^3.0.2",
                "phpunit/php-code-coverage": "^10.1.5",
                "phpunit/php-file-iterator": "^4.0",
                "phpunit/php-invoker": "^4.0",
                "phpunit/php-text-template": "^3.0",
                "phpunit/php-timer": "^6.0",
                "sebastian/cli-parser": "^2.0",
                "sebastian/code-unit": "^2.0",
                "sebastian/comparator": "^5.0",
                "sebastian/diff": "^5.0",
                "sebastian/environment": "^6.0",
                "sebastian/exporter": "^5.1",
                "sebastian/global-state": "^6.0.1",
                "sebastian/object-enumerator": "^5.0",
                "sebastian/recursion-context": "^5.0",
                "sebastian/type": "^4.0",
                "sebastian/version": "^4.0"
            },
            "require-dev": {
                "ext-pdo": "*"
            },
            "autoload": {
                "classmap": [
                    "src/"
                ],
                "files": [
                    "src/Framework/Assert/Functions.php"
                ]
            }
        })
    }

    /// Guzzle HTTP client composer.json.
    #[must_use]
    pub fn guzzle_composer_json() -> Value {
        json!({
            "name": "guzzlehttp/guzzle",
            "type": "library",
            "description": "Guzzle is a PHP HTTP client library",
            "license": "MIT",
            "require": {
                "php": "^7.2.5 || ^8.0",
                "guzzlehttp/promises": "^1.5.3 || ^2.0.1",
                "guzzlehttp/psr7": "^1.9.1 || ^2.5.1",
                "psr/http-client": "^1.0",
                "symfony/deprecation-contracts": "^2.2 || ^3.0"
            },
            "require-dev": {
                "bamarni/composer-bin-plugin": "^1.8.1",
                "ext-curl": "*",
                "php-http/client-integration-tests": "dev-master#2c025848417c1135031fdf9c728ee53d0a7ceaee as 3.0.999",
                "php-http/message-factory": "^1.1",
                "phpunit/phpunit": "^8.5.29 || ^9.5.23",
                "psr/log": "^1.1 || ^2.0 || ^3.0"
            },
            "autoload": {
                "psr-4": {
                    "GuzzleHttp\\": "src/"
                },
                "files": [
                    "src/functions_include.php"
                ]
            }
        })
    }

    /// Monolog composer.json (logging library).
    #[must_use]
    pub fn monolog_composer_json() -> Value {
        json!({
            "name": "monolog/monolog",
            "type": "library",
            "description": "Sends your logs to files, sockets, inboxes, databases and various web services",
            "license": "MIT",
            "require": {
                "php": ">=8.1",
                "psr/log": "^2.0 || ^3.0"
            },
            "require-dev": {
                "aws/aws-sdk-php": "^3.0",
                "doctrine/couchdb": "~1.0@dev",
                "elasticsearch/elasticsearch": "^7 || ^8",
                "ext-json": "*",
                "graylog2/gelf-php": "^1.4.2 || ^2.0",
                "guzzlehttp/guzzle": "^7.4.5",
                "guzzlehttp/psr7": "^2.2",
                "mongodb/mongodb": "^1.8",
                "php-amqplib/php-amqplib": "~2.4 || ^3",
                "phpstan/phpstan": "^1.9",
                "phpstan/phpstan-deprecation-rules": "^1.0",
                "phpstan/phpstan-strict-rules": "^1.4",
                "phpunit/phpunit": "^10.1",
                "predis/predis": "^1.1 || ^2",
                "rollbar/rollbar": "^4.0",
                "ruflin/elastica": "^7",
                "symfony/mailer": "^5.4 || ^6",
                "symfony/mime": "^5.4 || ^6"
            },
            "autoload": {
                "psr-4": {
                    "Monolog\\": "src/Monolog"
                }
            }
        })
    }

    /// Doctrine ORM composer.json.
    #[must_use]
    pub fn doctrine_orm_composer_json() -> Value {
        json!({
            "name": "doctrine/orm",
            "type": "library",
            "description": "Object-Relational-Mapper for PHP",
            "license": "MIT",
            "require": {
                "php": "^8.1",
                "composer-runtime-api": "^2",
                "doctrine/cache": "^1.12.1 || ^2.1.1",
                "doctrine/collections": "^1.5 || ^2.1",
                "doctrine/common": "^3.0.3",
                "doctrine/dbal": "^3.6.0 || ^4",
                "doctrine/deprecations": "^0.5.3 || ^1",
                "doctrine/event-manager": "^1.2 || ^2",
                "doctrine/inflector": "^1.4 || ^2.0",
                "doctrine/instantiator": "^1.3 || ^2",
                "doctrine/lexer": "^2 || ^3",
                "doctrine/persistence": "^3.1.1",
                "psr/cache": "^1 || ^2 || ^3",
                "symfony/console": "^5.4 || ^6.0 || ^7.0",
                "symfony/var-exporter": "^6.3.9 || ^7.0"
            },
            "require-dev": {
                "doctrine/coding-standard": "^12.0",
                "phpbench/phpbench": "^1.0",
                "phpstan/phpstan": "1.10.35",
                "phpunit/phpunit": "^10.4",
                "psr/log": "^1 || ^2 || ^3",
                "squizlabs/php_codesniffer": "3.7.2",
                "symfony/cache": "^5.4 || ^6.2 || ^7.0",
                "symfony/yaml": "^3.4 || ^4.0 || ^5.0 || ^6.0 || ^7.0"
            },
            "autoload": {
                "psr-4": {
                    "Doctrine\\ORM\\": "src"
                }
            }
        })
    }

    /// Composer.json with complex version constraints.
    #[must_use]
    pub fn complex_constraints_composer_json() -> Value {
        json!({
            "name": "test/complex-constraints",
            "type": "project",
            "require": {
                "php": ">=7.4 <8.3",
                "package/caret": "^1.2.3",
                "package/tilde": "~1.2.3",
                "package/exact": "1.2.3",
                "package/range": ">=1.0 <2.0",
                "package/or": "^1.0 || ^2.0",
                "package/wildcard": "1.2.*",
                "package/dev": "dev-main",
                "package/stability": "1.0@beta",
                "package/branch-alias": "dev-main as 2.0.x-dev"
            }
        })
    }

    /// Composer.json with all autoload types.
    #[must_use]
    pub fn all_autoload_types_composer_json() -> Value {
        json!({
            "name": "test/autoload-types",
            "type": "library",
            "autoload": {
                "psr-4": {
                    "App\\": "src/",
                    "App\\Sub\\": ["src/sub/", "src/other/"]
                },
                "psr-0": {
                    "Legacy_": "legacy/",
                    "OldStyle_": ["old/", "compat/"]
                },
                "classmap": [
                    "lib/",
                    "extra/SomeClass.php"
                ],
                "files": [
                    "src/helpers.php",
                    "src/functions.php"
                ],
                "exclude-from-classmap": [
                    "/tests/",
                    "/test/",
                    "/Tests/",
                    "/Test/"
                ]
            },
            "autoload-dev": {
                "psr-4": {
                    "Tests\\": "tests/"
                }
            }
        })
    }

    /// Simple composer.lock fixture.
    #[must_use]
    pub fn simple_composer_lock() -> Value {
        json!({
            "_readme": [
                "This file locks the dependencies of your project to a known state"
            ],
            "content-hash": "a1b2c3d4e5f6789012345678901234567890abcd",
            "packages": [
                {
                    "name": "monolog/monolog",
                    "version": "3.4.0",
                    "source": {
                        "type": "git",
                        "url": "https://github.com/Seldaek/monolog.git",
                        "reference": "e2392369686d420ca32df3803de28b5d6f76867d"
                    },
                    "dist": {
                        "type": "zip",
                        "url": "https://api.github.com/repos/Seldaek/monolog/zipball/e2392369686d420ca32df3803de28b5d6f76867d",
                        "reference": "e2392369686d420ca32df3803de28b5d6f76867d",
                        "shasum": ""
                    },
                    "require": {
                        "php": ">=8.1",
                        "psr/log": "^2.0 || ^3.0"
                    },
                    "type": "library",
                    "autoload": {
                        "psr-4": {
                            "Monolog\\": "src/Monolog"
                        }
                    },
                    "license": ["MIT"],
                    "description": "Sends your logs to files, sockets, inboxes, databases and various web services"
                },
                {
                    "name": "psr/log",
                    "version": "3.0.0",
                    "source": {
                        "type": "git",
                        "url": "https://github.com/php-fig/log.git",
                        "reference": "fe5ea303b0887d5caefd3d431c3e61ad47037001"
                    },
                    "dist": {
                        "type": "zip",
                        "url": "https://api.github.com/repos/php-fig/log/zipball/fe5ea303b0887d5caefd3d431c3e61ad47037001",
                        "reference": "fe5ea303b0887d5caefd3d431c3e61ad47037001",
                        "shasum": ""
                    },
                    "require": {
                        "php": ">=8.0.0"
                    },
                    "type": "library",
                    "autoload": {
                        "psr-4": {
                            "Psr\\Log\\": "src"
                        }
                    },
                    "license": ["MIT"],
                    "description": "Common interface for logging libraries"
                }
            ],
            "packages-dev": [
                {
                    "name": "phpunit/phpunit",
                    "version": "10.3.0",
                    "source": {
                        "type": "git",
                        "url": "https://github.com/sebastianbergmann/phpunit.git",
                        "reference": "abc123def456"
                    },
                    "dist": {
                        "type": "zip",
                        "url": "https://api.github.com/repos/sebastianbergmann/phpunit/zipball/abc123def456",
                        "reference": "abc123def456",
                        "shasum": ""
                    },
                    "require": {
                        "php": ">=8.1"
                    },
                    "type": "library"
                }
            ],
            "aliases": [],
            "minimum-stability": "stable",
            "stability-flags": [],
            "prefer-stable": true,
            "prefer-lowest": false,
            "platform": {
                "php": ">=8.1"
            },
            "platform-dev": [],
            "plugin-api-version": "2.6.0"
        })
    }

    /// Packagist API response for a package.
    #[must_use]
    pub fn packagist_package_response(name: &str) -> Value {
        let parts: Vec<&str> = name.split('/').collect();
        let vendor = parts.first().unwrap_or(&"vendor");
        let package = parts.get(1).unwrap_or(&"package");

        json!({
            "package": {
                "name": name,
                "description": format!("{} package", package),
                "time": "2024-01-01T00:00:00+00:00",
                "maintainers": [
                    {
                        "name": "maintainer",
                        "avatar_url": "https://www.gravatar.com/avatar/xxx"
                    }
                ],
                "versions": {
                    "1.0.0": {
                        "name": name,
                        "description": format!("{} package", package),
                        "version": "1.0.0",
                        "version_normalized": "1.0.0.0",
                        "license": ["MIT"],
                        "source": {
                            "type": "git",
                            "url": format!("https://github.com/{}/{}.git", vendor, package),
                            "reference": "abc123"
                        },
                        "dist": {
                            "type": "zip",
                            "url": format!("https://api.github.com/repos/{}/{}/zipball/v1.0.0", vendor, package),
                            "reference": "abc123",
                            "shasum": "sha256:abcdef1234567890"
                        },
                        "require": {
                            "php": ">=7.4"
                        },
                        "time": "2024-01-01T00:00:00+00:00",
                        "type": "library",
                        "autoload": {
                            "psr-4": {
                                format!("{}\\{}\\", vendor.to_uppercase(), package.to_uppercase()): "src/"
                            }
                        }
                    },
                    "2.0.0": {
                        "name": name,
                        "description": format!("{} package v2", package),
                        "version": "2.0.0",
                        "version_normalized": "2.0.0.0",
                        "license": ["MIT"],
                        "source": {
                            "type": "git",
                            "url": format!("https://github.com/{}/{}.git", vendor, package),
                            "reference": "def456"
                        },
                        "dist": {
                            "type": "zip",
                            "url": format!("https://api.github.com/repos/{}/{}/zipball/v2.0.0", vendor, package),
                            "reference": "def456",
                            "shasum": "sha256:fedcba0987654321"
                        },
                        "require": {
                            "php": ">=8.0"
                        },
                        "time": "2024-06-01T00:00:00+00:00",
                        "type": "library",
                        "autoload": {
                            "psr-4": {
                                format!("{}\\{}\\", vendor.to_uppercase(), package.to_uppercase()): "src/"
                            }
                        }
                    }
                },
                "type": "library",
                "repository": format!("https://github.com/{}/{}", vendor, package),
                "github_stars": 100,
                "github_watchers": 10,
                "github_forks": 20,
                "github_open_issues": 5,
                "language": "PHP",
                "dependents": 50,
                "suggesters": 5,
                "downloads": {
                    "total": 1000000,
                    "monthly": 50000,
                    "daily": 2000
                },
                "favers": 200
            }
        })
    }

    /// Security advisory response.
    #[must_use]
    pub fn security_advisory_response() -> Value {
        json!({
            "advisories": {
                "symfony/http-kernel": [
                    {
                        "advisoryId": "PKSA-1234-abcd",
                        "packageName": "symfony/http-kernel",
                        "affectedVersions": ">=4.0,<4.4.50|>=5.0,<5.4.20|>=6.0,<6.0.20|>=6.1,<6.1.12|>=6.2,<6.2.6",
                        "title": "CVE-2023-12345: Security vulnerability in HttpKernel",
                        "cve": "CVE-2023-12345",
                        "link": "https://symfony.com/blog/cve-2023-12345",
                        "reportedAt": "2023-06-15T00:00:00+00:00",
                        "sources": [
                            {
                                "name": "symfony",
                                "remoteId": "CVE-2023-12345"
                            }
                        ]
                    }
                ]
            }
        })
    }

    /// PHP class file content for autoloader testing.
    #[must_use]
    pub fn php_class_content(namespace: &str, class_name: &str) -> String {
        format!(
            r"<?php

declare(strict_types=1);

namespace {namespace};

use InvalidArgumentException;

/**
 * {class_name} class.
 */
class {class_name}
{{
    private string $name;
    private int $value;

    public function __construct(string $name, int $value = 0)
    {{
        if (empty($name)) {{
            throw new InvalidArgumentException('Name cannot be empty');
        }}
        $this->name = $name;
        $this->value = $value;
    }}

    public function getName(): string
    {{
        return $this->name;
    }}

    public function getValue(): int
    {{
        return $this->value;
    }}

    public function setValue(int $value): self
    {{
        $this->value = $value;
        return $this;
    }}
}}
"
        )
    }

    /// PHP interface content.
    #[must_use]
    pub fn php_interface_content(namespace: &str, interface_name: &str) -> String {
        format!(
            r"<?php

declare(strict_types=1);

namespace {namespace};

/**
 * {interface_name} interface.
 */
interface {interface_name}
{{
    public function execute(): void;
    public function getResult(): mixed;
}}
"
        )
    }

    /// PHP trait content.
    #[must_use]
    pub fn php_trait_content(namespace: &str, trait_name: &str) -> String {
        format!(
            r#"<?php

declare(strict_types=1);

namespace {namespace};

/**
 * {trait_name} trait.
 */
trait {trait_name}
{{
    protected function log(string $message): void
    {{
        echo "[LOG] {{$message}}\n";
    }}

    abstract protected function process(): void;
}}
"#
        )
    }

    /// PHP enum content (PHP 8.1+).
    #[must_use]
    pub fn php_enum_content(namespace: &str, enum_name: &str) -> String {
        format!(
            r"<?php

declare(strict_types=1);

namespace {namespace};

/**
 * {enum_name} enum.
 */
enum {enum_name}: string
{{
    case Pending = 'pending';
    case Active = 'active';
    case Completed = 'completed';
    case Failed = 'failed';

    public function label(): string
    {{
        return match($this) {{
            self::Pending => 'Pending',
            self::Active => 'Active',
            self::Completed => 'Completed',
            self::Failed => 'Failed',
        }};
    }}
}}
"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_composer_json() {
        let json = Fixtures::empty_composer_json();
        assert!(json["name"].is_string());
        assert!(json["require"].is_object());
    }

    #[test]
    fn test_laravel_composer_json_structure() {
        let json = Fixtures::laravel_composer_json();
        assert_eq!(json["name"], "laravel/laravel");
        assert!(json["require"]["laravel/framework"].is_string());
        assert!(json["autoload"]["psr-4"].is_object());
    }

    #[test]
    fn test_packagist_response_contains_versions() {
        let response = Fixtures::packagist_package_response("vendor/package");
        assert!(response["package"]["versions"]["1.0.0"].is_object());
        assert!(response["package"]["versions"]["2.0.0"].is_object());
    }

    #[test]
    fn test_php_class_content_valid() {
        let content = Fixtures::php_class_content("App\\Models", "User");
        assert!(content.contains("namespace App\\Models;"));
        assert!(content.contains("class User"));
    }
}
