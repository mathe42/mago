<?php

require_once __DIR__ . '/order_service.php';

class ReportService extends BaseService {
    public function process(): void {
        $this->log_action("Generating report");
    }

    public function get_revenue_report(): array {
        // Embedded SQL — should be detected via heuristic (starts with SELECT)
        return $this->db_query("SELECT DATE(created_at) as day, SUM(total) as revenue FROM orders WHERE created_at >= '2024-01-01' GROUP BY DATE(created_at)");
    }

    public function run_cleanup(): void {
        // Embedded Bash — should be detected
        exec("rm -rf /tmp/reports/*.csv");
        shell_exec("find /var/log -name '*.log' -mtime +30 -delete");
    }
}
