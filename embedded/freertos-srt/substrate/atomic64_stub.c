/* 64-bit atomic libcalls for single-core Cortex-M4.
 *
 * libsrt uses std::atomic<int64_t>/<uint64_t> (bandwidth/seqno counters). The
 * Cortex-M4 has no native 64-bit atomic instructions, so GCC lowers these to
 * __atomic_*_8 libcalls — normally satisfied by libatomic, which the bare-metal
 * arm-none-eabi multilib for cortex-m4f does NOT ship. On a single core, a
 * 64-bit atomic is just a critical section: mask interrupts (PRIMASK), do the
 * op, restore. No SMP, so this is correct here. */
#include <stdint.h>
#include <stddef.h>

static inline uint32_t irq_save(void)
{
    uint32_t primask;
    __asm volatile("mrs %0, primask\n\tcpsid i" : "=r"(primask) :: "memory");
    return primask;
}
static inline void irq_restore(uint32_t primask)
{
    __asm volatile("msr primask, %0" :: "r"(primask) : "memory");
}

uint64_t __atomic_load_8(const volatile void* ptr, int memorder)
{
    (void)memorder;
    uint32_t s = irq_save();
    uint64_t v = *(const volatile uint64_t*)ptr;
    irq_restore(s);
    return v;
}

void __atomic_store_8(volatile void* ptr, uint64_t val, int memorder)
{
    (void)memorder;
    uint32_t s = irq_save();
    *(volatile uint64_t*)ptr = val;
    irq_restore(s);
}

uint64_t __atomic_exchange_8(volatile void* ptr, uint64_t val, int memorder)
{
    (void)memorder;
    uint32_t s = irq_save();
    uint64_t old = *(volatile uint64_t*)ptr;
    *(volatile uint64_t*)ptr = val;
    irq_restore(s);
    return old;
}

uint64_t __atomic_fetch_add_8(volatile void* ptr, uint64_t val, int memorder)
{
    (void)memorder;
    uint32_t s = irq_save();
    uint64_t old = *(volatile uint64_t*)ptr;
    *(volatile uint64_t*)ptr = old + val;
    irq_restore(s);
    return old;
}

uint64_t __atomic_fetch_sub_8(volatile void* ptr, uint64_t val, int memorder)
{
    (void)memorder;
    uint32_t s = irq_save();
    uint64_t old = *(volatile uint64_t*)ptr;
    *(volatile uint64_t*)ptr = old - val;
    irq_restore(s);
    return old;
}

int __atomic_compare_exchange_8(volatile void* ptr, void* expected,
                                uint64_t desired, int weak,
                                int success, int failure)
{
    (void)weak; (void)success; (void)failure;
    uint32_t s = irq_save();
    uint64_t cur = *(volatile uint64_t*)ptr;
    int ok = (cur == *(uint64_t*)expected);
    if (ok)
        *(volatile uint64_t*)ptr = desired;
    else
        *(uint64_t*)expected = cur;
    irq_restore(s);
    return ok;
}
