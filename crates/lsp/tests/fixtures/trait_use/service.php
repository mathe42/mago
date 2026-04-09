<?php

require_once __DIR__ . '/loggable.php';

class UserService {
    use Loggable;
    use Cacheable;

    public function find_user(int $id): ?array {
        $cached = $this->cache_get("user_{$id}");
        if ($cached !== null) {
            $this->log("Cache hit for user {$id}");
            return $cached;
        }

        $this->debug("Loading user {$id} from database");
        $user = ['id' => $id, 'name' => 'John'];
        $this->cache_set("user_{$id}", $user);
        return $user;
    }

    public function delete_user(int $id): bool {
        $this->log("Deleting user {$id}");
        return true;
    }
}

$svc = new UserService();
$svc->find_user(1);
// $svc-> should complete: find_user, delete_user, log, debug, cache_get, cache_set
