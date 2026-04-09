<?php

trait Loggable {
    public function log(string $message): void {
        echo "[LOG] {$message}\n";
    }

    public function debug(string $message): void {
        echo "[DEBUG] {$message}\n";
    }
}

trait Cacheable {
    private array $cache = [];

    public function cache_get(string $key): mixed {
        return $this->cache[$key] ?? null;
    }

    public function cache_set(string $key, mixed $value): void {
        $this->cache[$key] = $value;
    }
}
