<?php

class Database {
    private string $host;
    private string $dbname;

    public function __construct(string $host, string $dbname) {
        $this->host = $host;
        $this->dbname = $dbname;
    }

    public function db_query(string $sql): array {
        return [];
    }

    public function db_select(string $table, string $where = ""): array {
        $sql = "SELECT * FROM {$table}";
        if ($where) {
            $sql .= " WHERE {$where}";
        }
        return $this->db_query($sql);
    }

    public function db_insert(string $table, array $data): int {
        return 0;
    }

    public function db_update(string $table, array $data, string $where): bool {
        return true;
    }

    public function db_delete(string $table, string $where): bool {
        return true;
    }

    public function escape(string $value): string {
        return addslashes($value);
    }
}
