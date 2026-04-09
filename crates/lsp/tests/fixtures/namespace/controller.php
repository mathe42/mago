<?php

namespace App\Controllers;

use App\Models\User;
use App\Models\Order;

class UserController {
    public function show(int $id): string {
        $user = User::find($id);
        if ($user === null) {
            return "User not found";
        }

        return $user->get_display_name();
        // $user-> should complete: get_display_name, id, name, email
    }

    public function create_order(int $user_id, float $amount): Order {
        $user = new User($user_id, "Test", "test@example.com");
        return new Order(1, $user, $amount);
    }
}
