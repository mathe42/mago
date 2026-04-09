<?php

require_once __DIR__ . '/base_service.php';

class OrderService extends BaseService {
    private int $user_id;

    public function __construct(string $host, string $dbname, int $user_id) {
        parent::__construct($host, $dbname);
        $this->user_id = $user_id;
    }

    public function process(): void {
        $this->log_action("Processing orders for user {$this->user_id}");
        $orders = $this->db_select("orders", "user_id = {$this->user_id}");
        foreach ($orders as $order) {
            $this->handle_order($order);
        }
    }

    public function handle_order(array $order): void {
        $this->log_action("Handling order {$order['id']}");
        $this->db_update("orders", ['status' => 'processed'], "id = {$order['id']}");
    }

    public function get_total(): float {
        $rows = $this->db_query("SELECT SUM(total) as sum FROM orders WHERE user_id = {$this->user_id}");
        return (float)($rows[0]['sum'] ?? 0);
    }
}

$svc = new OrderService("localhost", "mydb", 42);
$svc->process();
// $svc-> should complete: process, handle_order, get_total,
//   db_query, db_select, db_insert, db_update, db_delete, escape,
//   get_config, set_config, log_action
