//! Benchmarks for the plugin system.
//!
//! Performance targets:
//! - Plugin invocation overhead: <10ms
//! - Support for 20+ simultaneous plugins

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use libretto_plugin_system::{
    EventBus, EventContext, EventResult, Hook, HookRegistry, PluginDiscovery, Sandbox,
    SandboxConfig,
};
use std::path::PathBuf;
use std::time::Duration;

/// Benchmark hook registry operations.
fn bench_hook_registry(c: &mut Criterion) {
    let mut group = c.benchmark_group("hook_registry");

    // Benchmark registration
    group.bench_function("register_single", |b| {
        let registry = HookRegistry::new();
        b.iter(|| {
            registry.register(Hook::PreInstallCmd, black_box("test-plugin"), 0);
        });
    });

    // Benchmark lookup with different numbers of handlers
    for num_handlers in [1, 5, 10, 20, 50] {
        group.bench_with_input(
            BenchmarkId::new("get_handlers", num_handlers),
            &num_handlers,
            |b, &n| {
                let registry = HookRegistry::new();
                for i in 0..n {
                    registry.register(Hook::PreInstallCmd, format!("plugin-{i}"), i);
                }

                b.iter(|| {
                    black_box(registry.get_handlers(&Hook::PreInstallCmd));
                });
            },
        );
    }

    group.finish();
}

/// Benchmark event bus operations.
fn bench_event_bus(c: &mut Criterion) {
    let mut group = c.benchmark_group("event_bus");

    // Benchmark publish with different subscriber counts
    for num_subscribers in [1, 5, 10, 20] {
        group.bench_with_input(
            BenchmarkId::new("publish", num_subscribers),
            &num_subscribers,
            |b, &n| {
                let bus = EventBus::new(1000);
                let mut subs = Vec::new();
                for i in 0..n {
                    subs.push(bus.subscribe_to_topic(format!("plugin-{i}"), "*"));
                }

                b.iter(|| {
                    let msg = libretto_plugin_system::EventMessage::broadcast(
                        "source",
                        "test.topic",
                        libretto_plugin_system::MessagePayload::Empty,
                    );
                    bus.publish(msg).unwrap();
                    black_box(());
                });

                // Drain messages
                for sub in &subs {
                    let _ = sub.drain();
                }
            },
        );
    }

    group.finish();
}

/// Benchmark sandbox operations.
fn bench_sandbox(c: &mut Criterion) {
    let mut group = c.benchmark_group("sandbox");

    // Benchmark path checking
    group.bench_function("is_read_allowed", |b| {
        let mut config = SandboxConfig::default();
        config.allowed_read_paths = vec![
            PathBuf::from("/project"),
            PathBuf::from("/home/user/.composer"),
            PathBuf::from("/var/cache/libretto"),
        ];
        let sandbox = Sandbox::new(config);

        b.iter(|| {
            black_box(
                sandbox.is_read_allowed("plugin", std::path::Path::new("/project/src/file.php")),
            );
        });
    });

    // Benchmark network host checking
    group.bench_function("is_network_allowed", |b| {
        let mut config = SandboxConfig::default();
        config.allowed_hosts = vec![
            "packagist.org".to_string(),
            "*.github.com".to_string(),
            "*.githubusercontent.com".to_string(),
        ];
        let sandbox = Sandbox::new(config);

        b.iter(|| {
            black_box(sandbox.is_network_allowed("plugin", "api.github.com"));
        });
    });

    group.finish();
}

/// Benchmark event context creation.
fn bench_event_context(c: &mut Criterion) {
    let mut group = c.benchmark_group("event_context");

    group.bench_function("create_context", |b| {
        use libretto_core::PackageId;

        b.iter(|| {
            let ctx = EventContext::new()
                .with_operation("install")
                .with_project_root("/project")
                .with_vendor_dir("/project/vendor")
                .with_dev_mode(true)
                .with_packages(vec![
                    PackageId::parse("symfony/console").unwrap(),
                    PackageId::parse("doctrine/orm").unwrap(),
                    PackageId::parse("laravel/framework").unwrap(),
                ]);
            black_box(ctx);
        });
    });

    group.finish();
}

/// Benchmark simulated plugin invocation overhead.
fn bench_plugin_invocation(c: &mut Criterion) {
    let mut group = c.benchmark_group("plugin_invocation");
    group.measurement_time(Duration::from_secs(10));

    // Simulate the overhead of invoking a plugin (without actual plugin code)
    group.bench_function("invocation_overhead", |b| {
        let registry = HookRegistry::new();
        let sandbox = Sandbox::new(SandboxConfig::default());

        // Register 20 plugins (target: support 20+ plugins)
        for i in 0..20 {
            registry.register(Hook::PreInstallCmd, format!("plugin-{i}"), i);
        }

        b.iter(|| {
            // Get handlers (sorted by priority)
            let handlers = registry.get_handlers(&Hook::PreInstallCmd);

            // Create context
            let ctx = EventContext::new().with_operation("install");

            // Simulate checking each handler
            for handler in &handlers {
                // Check sandbox permissions
                let _ =
                    sandbox.is_read_allowed(&handler.plugin_id, std::path::Path::new("/project"));
                let _ = sandbox.is_network_allowed(&handler.plugin_id, "packagist.org");

                // Create result
                let result = EventResult::ok();
                black_box(&result);

                if !result.continue_processing {
                    break;
                }
            }

            black_box(ctx);
        });
    });

    group.finish();
}

/// Benchmark discovery cache performance.
fn bench_discovery_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("discovery_cache");

    group.bench_function("cache_lookup", |b| {
        let discovery = PluginDiscovery::new();

        b.iter(|| {
            black_box(discovery.find_by_name("symfony/flex"));
            black_box(discovery.find_by_name("composer/installers"));
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_hook_registry,
    bench_event_bus,
    bench_sandbox,
    bench_event_context,
    bench_plugin_invocation,
    bench_discovery_cache,
);

criterion_main!(benches);
