<?php

namespace App\Models;

class User {
    public int $id;
    public string $name;
    public string $email;

    public function __construct(int $id, string $name, string $email) {
        $this->id = $id;
        $this->name = $name;
        $this->email = $email;
    }

    public function get_display_name(): string {
        return "{$this->name} <{$this->email}>";
    }

    public static function find(int $id): ?self {
        return null;
    }
}

class Order {
    public int $id;
    public User $user;
    public float $total;

    public function __construct(int $id, User $user, float $total) {
        $this->id = $id;
        $this->user = $user;
        $this->total = $total;
    }

    public function get_summary(): string {
        return "Order #{$this->id} for {$this->user->name}: {$this->total}";
    }
}
