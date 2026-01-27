//! Benchmarks for the autoloader module.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use libretto_autoloader::{PhpParser, Scanner};

fn bench_php_parser(c: &mut Criterion) {
    let mut group = c.benchmark_group("php_parser");

    let simple_class = r#"<?php
namespace App\Models;

class User {
    public string $name;
    public string $email;

    public function __construct(string $name, string $email) {
        $this->name = $name;
        $this->email = $email;
    }
}
"#;

    let complex_file = r#"<?php
namespace App\Services;

use App\Models\User;
use App\Contracts\UserRepositoryInterface;

interface Authenticatable {
    public function authenticate(): bool;
}

trait HasTimestamps {
    public function touch(): void {}
}

enum UserStatus: string {
    case Active = 'active';
    case Inactive = 'inactive';
}

class UserService implements Authenticatable {
    use HasTimestamps;

    private UserRepositoryInterface $repository;

    public function __construct(UserRepositoryInterface $repo) {
        $this->repository = $repo;
    }

    public function authenticate(): bool {
        return true;
    }
}
"#;

    group.throughput(Throughput::Bytes(simple_class.len() as u64));
    group.bench_function("simple_class", |b| {
        let mut parser = PhpParser::new();
        b.iter(|| black_box(parser.parse_str(simple_class)))
    });

    group.throughput(Throughput::Bytes(complex_file.len() as u64));
    group.bench_function("complex_file", |b| {
        let mut parser = PhpParser::new();
        b.iter(|| black_box(parser.parse_str(complex_file)))
    });

    group.finish();
}

fn bench_scanner_creation(c: &mut Criterion) {
    c.bench_function("scanner_creation", |b| {
        b.iter(|| black_box(Scanner::without_exclusions()))
    });
}

criterion_group!(benches, bench_php_parser, bench_scanner_creation);
criterion_main!(benches);
