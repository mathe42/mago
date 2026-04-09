<?php

function greet(string $name): string {
    return "Hello, " . $name;
}

$message = greet("World");
echo $message;
