<?php

require_once __DIR__ . '/db.php';

abstract class BaseService extends Database {
    protected array $config = [];

    public function __construct(string $host, string $dbname) {
        parent::__construct($host, $dbname);
    }

    abstract public function process(): void;

    public function get_config(string $key): mixed {
        return $this->config[$key] ?? null;
    }

    public function set_config(string $key, mixed $value): void {
        $this->config[$key] = $value;
    }

    protected function log_action(string $action): void {
        $now = date("Y-m-d H:i:s");
        echo "[{$now}] {$action}\n";
    }
}
