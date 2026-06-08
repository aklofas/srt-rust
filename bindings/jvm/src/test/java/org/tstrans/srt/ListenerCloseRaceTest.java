package org.tstrans.srt;

import static org.junit.jupiter.api.Assertions.*;

import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.Timeout;
import org.tstrans.SrtException;

/**
 * Memory-safety stress test for the {@link Listener#close()} / {@link Listener#accept(Integer)}
 * race.
 *
 * <p>Before the lifetime fix, {@code close()} freed the {@code Box<Listener>} while another
 * thread was parked inside {@code srt_accept}. The {@code srt_close} fired by close woke the
 * parked accept, which then read its (now-freed) {@code accepted_*_timeout} fields → a
 * use-after-free that intermittently crashed the JVM worker with SIGSEGV.
 *
 * <p>The loop builds a listener on an ephemeral port, parks a thread in {@code accept()}, lets
 * it reach {@code srt_accept}, then calls {@code close()} from the main thread. After the fix,
 * close cancels the parked accept (it returns {@code CLOSED}) and only frees the allocation once
 * the parked accept has released the internal lock — no thread ever touches freed memory.
 */
final class ListenerCloseRaceTest {

    @Test
    @Timeout(30)
    void closeWhileAcceptParkedIsMemorySafe() throws Exception {
        for (int i = 0; i < 50; i++) {
            Listener listener =
                new Builder("srt://127.0.0.1:0?mode=listener").listener().listen();

            // Signalled by the accept thread immediately before it calls accept(), so the
            // main thread can bound-wait until accept is about to (or has) parked in srt_accept.
            CountDownLatch aboutToAccept = new CountDownLatch(1);
            AtomicReference<Throwable> unexpected = new AtomicReference<>();

            Thread acc =
                new Thread(
                    () -> {
                        aboutToAccept.countDown();
                        try {
                            // Blocks in srt_accept until close() wakes it. srt_accept reads
                            // self.handle out of the listener struct, and the success path would
                            // read self.accepted_*_timeout — the exact memory the pre-fix close()
                            // frees out from under this parked call.
                            Socket s = listener.accept(null);
                            // No peer connects, so a successful accept is itself a bug.
                            s.close();
                            unexpected.set(new AssertionError("accept returned a Socket"));
                        } catch (SrtException e) {
                            // CLOSED is the sanctioned wake outcome; anything else is unexpected
                            // but not, in itself, a memory-safety failure.
                            if (e.kind() != SrtException.Kind.CLOSED) {
                                unexpected.set(e);
                            }
                        } catch (Throwable t) {
                            unexpected.set(t);
                        }
                    },
                    "accept-" + i);
            acc.start();

            // Wait until the accept thread is at the brink of srt_accept, then give it a brief
            // window to actually enter the blocking call. (Thread.sleep here is in the JVM, not
            // the shell — the sandbox sleep restriction does not apply.)
            assertTrue(aboutToAccept.await(2, TimeUnit.SECONDS), "accept thread never started");
            Thread.sleep(20);

            // The race under test: free the listener while accept is parked. Under the pre-fix
            // close() this freed the Box<Listener> the parked accept still dereferences (UAF);
            // after the fix close() cancels first and frees only once the parked accept has
            // released the internal lock.
            listener.close();

            acc.join(2000);
            assertFalse(acc.isAlive(), "accept thread did not unblock after close (run " + i + ")");
            assertNull(unexpected.get(), "accept thread saw an unexpected failure (run " + i + ")");
        }
    }

    /**
     * Close-vs-fresh-entry stress. With the leased {@code HandleRegistry}, a thread that
     * keeps calling a normal native getter ({@code localAddr()}) while another thread calls
     * {@code close()} must never crash the JVM; the only sanctioned exception the busy thread
     * may observe is {@link IllegalStateException} (handle claimed/closed). This is the case
     * round-1 declared unsafe (close racing a fresh entry); the registry now makes it
     * deterministically memory-safe.
     */
    @Test
    @Timeout(30)
    void closeVsFreshEntryIsMemorySafe() throws Exception {
        for (int i = 0; i < 200; i++) {
            Listener listener =
                new Builder("srt://127.0.0.1:0?mode=listener").listener().listen();

            CountDownLatch ready = new CountDownLatch(1);
            AtomicReference<Throwable> unexpected = new AtomicReference<>();

            Thread caller =
                new Thread(
                    () -> {
                        ready.countDown();
                        // Hammer a cheap, non-blocking native getter. Each call reads the
                        // current registry id and leases it; a concurrent close() either lets
                        // the call run or trips a clean IllegalStateException — never UB.
                        for (int k = 0; k < 1000; k++) {
                            try {
                                listener.localAddr();
                            } catch (IllegalStateException expected) {
                                // Handle was claimed by close() — sanctioned, keep going.
                            } catch (SrtException e) {
                                // localAddr can surface IO if the socket is mid-teardown; not a
                                // memory-safety failure, tolerate it.
                            } catch (Throwable t) {
                                unexpected.set(t);
                                return;
                            }
                        }
                    },
                    "caller-" + i);
            caller.start();

            // Let the caller get going, then race close() against its in-flight getter calls.
            assertTrue(ready.await(2, TimeUnit.SECONDS), "caller thread never started");
            listener.close();

            caller.join(5000);
            assertFalse(caller.isAlive(), "caller thread did not finish (run " + i + ")");
            assertNull(unexpected.get(), "caller thread saw an unexpected failure (run " + i + ")");
        }
    }
}
