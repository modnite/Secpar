package com.pcremote.prs

/**
 * Round trips on the current path and on the other one (direct or relay), five each, as the
 * desktop client keeps them. The other path wins when it is 30 % and 20 ms faster by the median.
 */
internal class PathTimes {
    private val current = ArrayDeque<Int>()
    private val other = ArrayDeque<Int>()

    fun current(ms: Int) = push(current, ms)
    fun other(ms: Int) = push(other, ms)
    fun clear() { current.clear(); other.clear() }

    fun otherFaster(): Boolean {
        if (current.size < 5 || other.size < 5) return false
        val cur = median(current)
        val oth = median(other)
        return oth * 10 < cur * 7 && cur >= oth + 20
    }

    private fun push(v: ArrayDeque<Int>, ms: Int) {
        if (v.size == 5) v.removeFirst()
        v.addLast(ms)
    }

    private fun median(v: ArrayDeque<Int>) = v.sorted()[v.size / 2]
}
