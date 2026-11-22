@file:Suppress("unused", "UNUSED_PARAMETER", "UNUSED_VARIABLE")

package com.example.kdemo

typealias IntList = List<Int>

interface Greeter {
    fun greet(name: String): String
    val label: String
}

open class Base(val id: Int) {
    open fun describe(): String = "base:$id"
    open val baseLabel: String get() = "base"
}

class Box<T : Any>(val value: T) {
    fun get(): T = value
    fun <R : Comparable<R>> map(f: (T) -> R): R = f(value)
    val nullable: T? = null
    companion object {
        const val KIND: String = "box"
        fun make(value: Int): Box<Int> = Box(value)
    }
}

data class Pair2<out A, out B>(val first: A, val second: B)

class Impl(override val label: String) : Greeter {
    override fun greet(name: String): String = "$label $name"
}

object Singleton {
    var counter: Int = 0
    fun bump(): Int { counter += 1; return counter }
}

enum class Color { RED, GREEN }

sealed class Shape {
    class Circle(val radius: Double) : Shape()
    object Empty : Shape()
}

class GenericBox<I, O>(private val input: I, private val output: O) {
    fun inValue(): I = input
    fun outValue(): O = output
}

class Producer<out T>(private val value: T) {
    fun get(): T = value
}

class Consumer<in T> {
    fun consume(value: T): Int = 0
}

fun projections(a: List<out Number>, b: MutableList<in Number>): Int = a.size

fun topLevel(x: Int): Int = x + 1

fun String.shout(): String = this.uppercase() + "!"

fun Int.times2(): Int = this * 2

suspend fun suspendWork(input: String): String = input

inline fun inlined(action: () -> Int): Int = action()

fun outer(): Int {
    fun local(a: Int): Int = a * 3
    return local(4)
}

fun nullable(x: String?): String? = x?.uppercase()

fun varargs(vararg xs: Int): Int = xs.size

fun stars(xs: List<*>): Int = xs.size

fun <T : Comparable<T>> maxOf2(a: T, b: T): T = if (a > b) a else b

fun <T> generic(vararg xs: T): List<T> = xs.toList()

fun consumeAny(value: Any): String = value.toString()

fun consumeString(value: String): String = value

fun genericBoxUse(): String {
    val box: GenericBox<String, Int> = GenericBox("a", 1)
    return box.inValue() + box.outValue()
}

suspend fun useAll(): String {
    val box = Box(1)
    val impl = Impl("hi")
    val pair: Pair2<String, Int> = Pair2("a", 1)
    val list: IntList = listOf(1, 2)
    val shape: Shape = Shape.Circle(1.0)
    Singleton.bump()
    val lambda = { x: Int -> x * 2 }
    val inlinedValue = inlined { lambda(21) }
    val local = outer()
    val text = impl.greet("bob").shout()
    val m = maxOf2(1, 2)
    val g = generic("a", "b")
    val sw = suspendWork("s")
    val nb = nullable(null)
    val va = varargs(1, 2)
    val st = stars(list)
    val t2 = 3.times2()
    val gb = genericBoxUse()
    val prod = Producer("p").get()
    val cons = Consumer<String>().consume("c")
    val proj = projections(listOf(1), mutableListOf<Number>())
    return "$box${box.get()}$pair$list$shape${Singleton.counter}$inlinedValue$local$text$m$g$sw$nb$va$st$t2$gb$prod$cons$proj"
}

fun main() {
    val kept = listOf(::suspendWork, ::useAll)
    println(kept)
    println(Base(1).describe())
    println(Box.make(2).value)
    println(Color.RED)
    println(topLevel(1))
    println(consumeAny(1))
    println(consumeString("x"))
}
