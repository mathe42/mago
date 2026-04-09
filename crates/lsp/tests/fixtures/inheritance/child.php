<?php

require_once __DIR__ . '/base.php';

class Dog extends Animal {
    private string $breed;

    public function __construct(string $name, int $age, string $breed) {
        parent::__construct($name, $age);
        $this->breed = $breed;
    }

    public function speak(): string {
        return "{$this->name} barks!";
    }

    public function fetch(string $item): string {
        return "{$this->name} fetches {$item}";
    }

    public function get_breed(): string {
        return $this->breed;
    }
}

$dog = new Dog("Rex", 5, "Shepherd");
echo $dog->speak();
// $dog-> should complete: speak, get_age, fetch, get_breed, name
