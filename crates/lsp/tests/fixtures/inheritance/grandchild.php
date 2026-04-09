<?php

require_once __DIR__ . '/child.php';

class GuideDog extends Dog {
    private bool $certified;

    public function __construct(string $name, int $age, string $breed, bool $certified) {
        parent::__construct($name, $age, $breed);
        $this->certified = $certified;
    }

    public function is_certified(): bool {
        return $this->certified;
    }

    public function guide(string $destination): string {
        return "{$this->name} guides to {$destination}";
    }
}

$guide = new GuideDog("Buddy", 3, "Labrador", true);
echo $guide->speak();
// $guide-> should complete: speak, get_age, fetch, get_breed, is_certified, guide, name
