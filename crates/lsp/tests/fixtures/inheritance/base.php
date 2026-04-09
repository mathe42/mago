<?php

class Animal {
    public string $name;
    protected int $age;

    public function __construct(string $name, int $age) {
        $this->name = $name;
        $this->age = $age;
    }

    public function speak(): string {
        return "{$this->name} makes a sound";
    }

    public function get_age(): int {
        return $this->age;
    }

    protected function internal_helper(): void {
        // do something
    }
}
