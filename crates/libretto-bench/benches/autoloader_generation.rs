//! Autoloader generation benchmarks.
//!
//! Comprehensive benchmarks for PHP autoloader with various project sizes.

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use libretto_autoloader::{AutoloadConfig, IncrementalCache, OptimizationLevel, Psr4Config};
use libretto_bench::fixtures::generate_php_class;
use libretto_bench::generators::PhpProjectGenerator;
use std::path::PathBuf;
use std::time::Duration;

/// Benchmark PHP class content parsing.
fn bench_php_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("autoloader/parse");

    let simple_class = generate_php_class("App", "SimpleClass");
    let complex_class = r"<?php
declare(strict_types=1);

namespace App\Services;

use App\Contracts\ServiceInterface;
use App\Traits\Loggable;
use App\Traits\Cacheable;
use Psr\Log\LoggerInterface;

/**
 * Complex service with multiple features.
 */
interface Processable {
    public function process(): void;
}

trait HasEvents {
    protected array $events = [];
    public function trigger(string $event): void {
        foreach ($this->events as $listener) {
            call_user_func($listener, $event);
        }
    }
}

enum Status: string {
    case Pending = 'pending';
    case Active = 'active';
    case Complete = 'complete';
}

abstract class BaseService implements ServiceInterface, Processable {
    use Loggable, Cacheable, HasEvents;
    
    protected Status $status = Status::Pending;
    protected LoggerInterface $logger;
    
    abstract protected function doProcess(): void;
    
    public function process(): void {
        $this->log('Processing started');
        $this->doProcess();
        $this->status = Status::Complete;
        $this->trigger('completed');
    }
}

class ComplexService extends BaseService {
    private array $handlers = [];
    private readonly string $name;
    
    public function __construct(
        private readonly string $serviceName,
        private readonly int $priority = 0,
    ) {
        $this->name = $serviceName;
    }
    
    protected function doProcess(): void {
        foreach ($this->handlers as $handler) {
            $handler->handle();
        }
    }
    
    public function addHandler(callable $handler): self {
        $this->handlers[] = $handler;
        return $this;
    }
}
";

    // Simple class parsing throughput
    group.throughput(Throughput::Bytes(simple_class.len() as u64));
    group.bench_function("simple_class", |b| {
        b.iter(|| {
            // Extract namespace and class using string matching (fast path)
            let content = black_box(&simple_class);
            let has_namespace = content.contains("namespace ");
            let has_class = content.contains("class ");
            black_box((has_namespace, has_class))
        });
    });

    // Complex class parsing throughput
    group.throughput(Throughput::Bytes(complex_class.len() as u64));
    group.bench_function("complex_class", |b| {
        b.iter(|| {
            let content = black_box(complex_class);
            let has_interface = content.contains("interface ");
            let has_trait = content.contains("trait ");
            let has_enum = content.contains("enum ");
            let has_abstract = content.contains("abstract class ");
            let has_class = content.contains("class ");
            black_box((has_interface, has_trait, has_enum, has_abstract, has_class))
        });
    });

    group.finish();
}

/// Benchmark `AutoloadConfig` creation and serialization.
fn bench_autoload_config(c: &mut Criterion) {
    let mut group = c.benchmark_group("autoloader/config");

    for num_namespaces in [5, 20, 50] {
        group.bench_with_input(
            BenchmarkId::new("create", num_namespaces),
            &num_namespaces,
            |b, &n| {
                b.iter(|| {
                    let mut psr4 = Psr4Config::default();
                    for i in 0..n {
                        psr4.mappings.insert(
                            format!("Vendor{}\\Package{}\\", i / 10, i),
                            vec![format!("vendor/vendor{}/package{}/src/", i / 10, i)],
                        );
                    }
                    let config = AutoloadConfig {
                        psr4,
                        ..Default::default()
                    };
                    black_box(config)
                });
            },
        );
    }

    // Benchmark serialization
    let mut psr4 = Psr4Config::default();
    for i in 0..20 {
        psr4.mappings.insert(
            format!("Vendor{}\\Package{}\\", i / 10, i),
            vec![format!("vendor/vendor{}/package{}/src/", i / 10, i)],
        );
    }
    let config = AutoloadConfig {
        psr4,
        ..Default::default()
    };

    group.bench_function("serialize_json", |b| {
        b.iter(|| {
            let json = serde_json::to_string(black_box(&config)).unwrap();
            black_box(json)
        });
    });

    group.finish();
}

/// Benchmark small project scanning (50-100 files).
fn bench_small_project(c: &mut Criterion) {
    let mut group = c.benchmark_group("autoloader/scan/small");
    group.sample_size(30);
    group.measurement_time(Duration::from_secs(15));

    for num_files in [50, 100] {
        let project_gen = PhpProjectGenerator::new().unwrap();
        project_gen.generate_files(num_files).unwrap();

        group.throughput(Throughput::Elements(num_files as u64));
        group.bench_with_input(
            BenchmarkId::new("files", num_files),
            project_gen.root(),
            |b, root| {
                b.iter(|| {
                    // Scan directory for PHP files
                    let mut count = 0;
                    for entry in walkdir::WalkDir::new(root)
                        .into_iter()
                        .filter_map(|e| e.ok())
                    {
                        if entry.path().extension().is_some_and(|ext| ext == "php") {
                            count += 1;
                        }
                    }
                    black_box(count)
                });
            },
        );
    }

    group.finish();
}

/// Benchmark medium project scanning (500-1000 files).
fn bench_medium_project(c: &mut Criterion) {
    let mut group = c.benchmark_group("autoloader/scan/medium");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(20));

    for num_files in [500, 1000] {
        let project_gen = PhpProjectGenerator::new().unwrap();
        project_gen.generate_files(num_files).unwrap();

        group.throughput(Throughput::Elements(num_files as u64));
        group.bench_with_input(
            BenchmarkId::new("files", num_files),
            project_gen.root(),
            |b, root| {
                b.iter(|| {
                    let mut count = 0;
                    for entry in walkdir::WalkDir::new(root)
                        .into_iter()
                        .filter_map(|e| e.ok())
                    {
                        if entry.path().extension().is_some_and(|ext| ext == "php") {
                            count += 1;
                        }
                    }
                    black_box(count)
                });
            },
        );
    }

    group.finish();
}

/// Benchmark large project scanning (5000-10000 files).
fn bench_large_project(c: &mut Criterion) {
    let mut group = c.benchmark_group("autoloader/scan/large");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));

    for num_files in [5000, 10000] {
        let project_gen = PhpProjectGenerator::new().unwrap();
        project_gen.generate_files(num_files).unwrap();

        group.throughput(Throughput::Elements(num_files as u64));
        group.bench_with_input(
            BenchmarkId::new("files", num_files),
            project_gen.root(),
            |b, root| {
                b.iter(|| {
                    // Parallel scanning using rayon
                    use rayon::prelude::*;

                    let files: Vec<_> = walkdir::WalkDir::new(root)
                        .into_iter()
                        .filter_map(|e| e.ok())
                        .filter(|e| e.path().extension().is_some_and(|ext| ext == "php"))
                        .collect();

                    // Parallel content reading
                    let results: Vec<_> = files
                        .par_iter()
                        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
                        .collect();

                    black_box(results.len())
                });
            },
        );
    }

    group.finish();
}

/// Benchmark `IncrementalCache` operations.
fn bench_incremental_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("autoloader/cache");
    group.sample_size(20);

    group.bench_function("create_cache", |b| {
        b.iter_with_setup(
            || tempfile::tempdir().unwrap(),
            |dir| {
                let cache_path = dir.path().join("autoload.cache");
                let cache = IncrementalCache::load_or_create(cache_path);
                black_box(cache)
            },
        );
    });

    group.bench_function("get_classmap_empty", |b| {
        let dir = tempfile::tempdir().unwrap();
        let cache = IncrementalCache::load_or_create(dir.path().join("autoload.cache"));

        b.iter(|| {
            let classmap = cache.get_classmap();
            black_box(classmap)
        });
    });

    group.finish();
}

/// Benchmark classmap generation.
fn bench_classmap_generation(c: &mut Criterion) {
    let mut group = c.benchmark_group("autoloader/classmap");
    group.sample_size(20);

    for num_classes in [100, 500, 1000] {
        // Pre-generate class data
        let classes: Vec<(String, PathBuf)> = (0..num_classes)
            .map(|i| {
                let namespace = format!("App\\Models\\Model{i}");
                let path = PathBuf::from(format!("src/Models/Model{i}.php"));
                (namespace, path)
            })
            .collect();

        group.throughput(Throughput::Elements(num_classes as u64));
        group.bench_with_input(
            BenchmarkId::new("classes", num_classes),
            &classes,
            |b, classes| {
                b.iter(|| {
                    // Generate PHP classmap array
                    let mut output = String::with_capacity(num_classes * 100);
                    output.push_str(
                        "<?php\n\n// autoload_classmap.php @generated\n\nreturn array(\n",
                    );

                    for (class_name, path) in classes {
                        output.push_str(&format!(
                            "    '{}' => $baseDir . '{}',\n",
                            class_name,
                            path.display()
                        ));
                    }

                    output.push_str(");\n");
                    black_box(output)
                });
            },
        );
    }

    group.finish();
}

/// Benchmark `OptimizationLevel` operations.
fn bench_optimization_levels(c: &mut Criterion) {
    let mut group = c.benchmark_group("autoloader/optimization");

    group.bench_function("level_comparison", |b| {
        b.iter(|| {
            let levels = [
                OptimizationLevel::None,
                OptimizationLevel::Optimized,
                OptimizationLevel::Authoritative,
            ];

            for level in &levels {
                black_box(level >= &OptimizationLevel::Optimized);
            }
        });
    });

    group.bench_function("from_int", |b| {
        b.iter(|| {
            for i in 0..3 {
                black_box(OptimizationLevel::from_int(i));
            }
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_php_parsing,
    bench_autoload_config,
    bench_small_project,
    bench_medium_project,
    bench_large_project,
    bench_incremental_cache,
    bench_classmap_generation,
    bench_optimization_levels,
);

criterion_main!(benches);
