<?php

/**
 * Begrüßt eine Person
 * @param $name test
 * @param $var asfjdkm,khjsdaf
 */
function greet(string $name, int $var): string
{
    return 'Hello, ' . $name;
}

$message = greet('World', 5);
echo $message;

trait TraitName
{
    /**
     * Hallo welt
     */
    function ccc(int $xxsdasd)
    {
        return 42;
    }
}

class DEF
{
    use TraitName;

    function bbb(int $xxsdasd)
    {
        return 42;
    }
}

/**
 * jasdhjshd
 */
class ABC extends DEF
{
    /**
     * Hallo welt
     */
    var string $v1;

    function __construct(string $aa)
    {
        $this->v1 = $aa;
        $this->bbb(42);
    }

    /**
     * Begrüßt eine Person
     * @param $a test34567
     */
    function aaa(int $b): ABC
    {
        $xx = $b;
        return new ABC('55555');
    }
}

$a = new ABC('kjfdsk');
$x = $a->aaa(5456);
$y = $a->v1;
$a->aaa(45);

$a->ccc(42);
