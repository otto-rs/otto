Below is a brief taxonomy of the most common approaches to scheduling a DAG of tasks onto a fixed number of “slots” (e.g. GNU Make’s --jobs), along with their core ideas. In practice, finding an optimal schedule to minimize total completion time (makespan) is NP-hard, so almost all build‐systems or parallel runtimes fall back on heuristics. The two broad families are:

    Simple greedy (online) scheduling

    Priority‐driven (list) scheduling with offline heuristics

Within (2) you’ll also occasionally see specialized graph‐partitioning or leveling algorithms (e.g. Coffman–Graham), but almost always wrapped into a list‐scheduling framework. Below each approach is named and its fundamentals sketched out.
1. Simple Greedy (Online) Scheduling
1.1 Kahn’s‐style “as soon as possible” Scheduler

    Name: Online FIFO (or “Kahn”) scheduling, employed in GNU Make, Ninja, etc.

    Core idea:

        Maintain a queue (ReadyQueue) of all tasks whose dependencies are satisfied (i.e.\ their in‐degree is zero).

        Whenever a job‐slot becomes free (a child finishes), pop the next task from ReadyQueue and start it.

        Once that task finishes, mark its dependent(s) as having one fewer unmet dependency; if any dependent’s in‐degree drops to zero, enqueue it into ReadyQueue.

        Repeat until all tasks have run.

    Characteristics:

        Online: doesn’t require any information about task “weights” or durations ahead of time. As soon as a slot is free, pick some ready node.

        Order: often FIFO (first‐in‐first‐out) or based on declaration order in the buildfile.

        Pros: trivial to implement; very low overhead; well suited when you don’t know or can’t estimate compile‐times.

        Cons: can lead to suboptimal makespan, especially if some long‐running tasks block smaller ones—there is no notion of “priority” or “longest‐path.”

Most make‐style implementations are variations of this. For instance, GNU Make maintains a pool of workers: whenever any worker is idle, it picks the next ready target (in some deterministic order, usually depth‐first or breadth‐first based on how the user wrote the Makefile). Because it doesn’t look ahead, two long‐paths in the DAG might be scheduled badly (e.g. both long‐tasks start at roughly the same time and cause other ready tasks to be delayed).
2. Priority‐Driven (“List”) Scheduling

In the “list scheduling” family, you still keep a set of ready tasks, but you assign each task a static or dynamic priority before (or during) execution, and always pick the ready task with highest priority when a slot frees up. The challenge is how to compute those priorities.
2.1 Pure “Static Level” / “Longest Path” Priority

    Name: Critical‐Path (or “Static Bottom‐Level”) list scheduling

    Core idea:

        Offline pass: Traverse the DAG from leaves back to roots. For each task t, compute
        blevel(t)  =  w(t)  +  max⁡(t→s)  blevel(s) ,
        blevel(t)=w(t)+(t→s)max​blevel(s),

        where w(t) is a heuristic “weight” (e.g. expected compile time, or just 1 if unknown), and the max is over all successors s. If you don’t know actual durations, you can treat every task as weight 1, so the bottom‐level is just the length (in edges) of the longest downstream path.

        Ready set: Maintain a priority queue of all ready tasks, keyed by descending blevel.

        Whenever a slot frees, pop the ready task with the largest blevel(t) and start it.

        Upon completion, update successors’ in‐degrees and push newly‐ready ones into the same priority queue.

    Fundamental aspects:

        By picking the task with the largest “longest‐path‐to‐exit,” you try to keep the global critical path busy, thus heuristically reduce global idle time.

        If you know task durations fairly accurately, w(t) can be actual expected run times; otherwise treat them uniformly.

        This is the classic “as soon as possible (ASAP) scheduling with critical‐path priority.”

    Pros:

        Tends to produce much better makespans than plain FIFO for graphs with unevenly sized subtrees.

        Still relatively cheap to implement: one offline pass to compute bottom‐levels, plus a binary‐heap (or multiset) for ready tasks.

    Cons:

        If weights w(t) are inaccurate or all 1, it still may not optimally balance the load.

        Completely offline: once priorities are computed, it never adapts during runtime if tasks run slower/faster than expected—though a dynamic version can recompute bottom‐levels online, at higher overhead.

2.2 Heterogeneous Earliest Finish‐Time (HEFT) / Modified Critical Path

    Name: HEFT (Heterogeneous Earliest Finish Time), rate‐monotonic variations, etc.

    Core idea:

        Weight annotation: Each edge (u→v) can carry an estimated communication cost (or “overhead”) if tasks land on different processors. Each node u has a computation cost c(u).

        Rankcing: Compute an upward rank for each node:
        ranku(t)=c(t)  +  max⁡(t→s){comm(t,s)  +  ranku(s)}.
        ranku​(t)=c(t)+(t→s)max​{comm(t,s)+ranku​(s)}.

        This is very similar to the bottom‐level/blevel above, but takes into account heterogeneity (network or slot‐specific speeds).

        Processor selection: When multiple slots might be heterogeneous (different speeds or I/O channels), each ready task is assigned to the processor (slot) that yields the earliest finish time (considering communication or data‐locality costs).

        Tie breaking: Always take the not‐yet‐scheduled ready task with maximum rank_u(t).

    Fundamental aspects:

        Designed for distributed/multi‐cluster DAG scheduling—considers inter‐task communication overhead.

        Works best when you have good static estimates of compute/communication times.

        Still a “list‐scheduling” heuristic: it never “un‐schedules” or reshuffles once started.

    Pros:

        For heterogeneous environments (e.g. CPU vs GPU, networked nodes), it’s more accurate than a uniform‐weight approach.

        Empirically yields good makespans on common scientific‐workflow DAGs.

    Cons:

        Requires build‐time annotation or profiling data for each task’s runtime and its data dependencies—often unavailable or inaccurate for everyday “build” tasks.

        Implementation complexity is higher (must keep track of data‐transfer times).

2.3 Coffman–Graham and Level‐Based Packing

    Name: Coffman–Graham (1972) or “Level Scheduling with width W”

    Core idea:

        Assign each node a “level‐label” in a way that respects the DAG partial order, but tries to pack each level into at most W nodes (where W is the number of slots).

        More concretely, you do a topological numbering, but whenever you assign labels to nodes with the same set of predecessor‐labels, you choose the node with the largest number of successors first. That way, you flatten the graph into “levels” such that no level has more than W nodes.

        Then schedule level 0 nodes in any order on W slots; once level 0 is done, schedule level 1, etc.

    Fundamental aspects:

        Aims to minimize the maximum “width” of any antichain in the DAG by reordering; if you can ensure no level has more than W tasks, you can (in theory) complete in a number of steps equal to the total number of levels.

        It’s an offline, static algorithm: on a DAG with fixed edges, you solve a labeling problem that tries to pack nodes as densely as possible subject to dependencies.

    Pros:

        Guaranteed to produce a schedule whose length is within a factor of 2 of optimal if all task durations are equal.

        Simple if your tasks are unit‐weight and you just want to minimize the number of “rounds.”

    Cons:

        Doesn’t handle heterogeneous durations easily (everyone is assumed weight 1).

        If you have actual run‐time variation, it isn’t robust: once a long task in level 0 delays everything, you can incur idle time in later levels.

3. “Dynamic” or Adaptive List Scheduling

If you want adaptivity when actual runtimes deviate from estimates, you can employ a hybrid:

    Recompute priorities online: After each task finishes, recompute bottom‐levels of the remaining subgraph (whatever hasn’t started yet). Then reinstate the ready tasks in a priority queue sorted by updated bottom‐levels. This is sometimes called Dynamic Critical‐Path or Work‐Stealing variants.

    Work‐Stealing Frameworks (e.g. Cilk, TBB, Rayon) maintain a global or per‐worker deque of ready tasks. When a worker becomes idle, it “steals” the highest‐priority task from another worker’s deque. They usually schedule by LIFO for cache locality, but can be tweaked to prioritize by approximate “height” in the DAG.

Fundamentals:

    Every time a task finishes, its dependents may become ready. Instead of enqueuing them at the back of a simple queue, you recalc a priority (bottom‐level or estimated remaining work) and push them into a prioritized structure.

    Helps when some tasks run unexpectedly longer/shorter than their static estimates.

    Slightly higher overhead: priorities must be recomputed or maintained, and stealing adds synchronization.

Trade-offs:

    Overhead vs Makespan: If tasks are extremely short (e.g. milliseconds), too much priority‐recomputation can hurt. If tasks are long (seconds+), the extra scheduling accuracy usually pays off.

    Most build‐systems do not do a full dynamic bottom‐level recomputation, because they don’t know task durations at all; they default to static or FIFO.

4. Offline + Online Hybrids

In some more advanced DAG runtimes, you do:

    Offline analysis (before the first task runs): assign each node a static priority (e.g. its bottom‐level under unit weights, or measured benchmark times).

    Online execution: maintain a ready priority queue by that static priority.

    When a task finishes, if you detect it took much longer/shorter than expected, you adjust priorities of its successors and bubble them up or down in the queue.

This is effectively a semi‐static list scheduler: most of the priority work is done in one offline pass, but you allow small adjustments if reality diverges from your model.
5. Summary of Names & When They’re Used

    Online FIFO (Kahn‐style)

        Used by: GNU Make, Ninja, most simple CI task‐runners.

        When: You have no reliable weights for tasks. You just want “as soon as possible” parallelism.

    Critical‐Path (Static Bottom‐Level) List Scheduling

        Also called: “Longest–path first,” “ASAP with bottom‐level priority.”

        Used by: Many scientific workflow engines (e.g. Pegasus, Kronos), some build systems (e.g. Bazel can use estimated times).

        When: You can approximate each task’s runtime (or assume unit weight). You want to reduce idle time on each worker.

    Coffman–Graham Level Scheduling

        Used in: Theoretical scheduling proofs, some educational tools.

        When: All tasks have equal weight and you want a 2‐approximation for makespan or a guaranteed scheduling width.

    HEFT (Heterogeneous Earliest Finish Time)

        Used by: Heterogeneous cluster schedulers, grid computing frameworks.

        When: You have known per‐task compute costs and known inter‐task communication overheads.

    Dynamic Priority / Work Stealing

        Used by: Cilk, Intel TBB, Rayon, Fork‐Join runtimes.

        When: You want to be robust to runtime variability, and your tasks form a user‐visible DAG (e.g. recursive divide‐and‐conquer).

6. “Make’s Default” Behavior in a Nutshell

If you just care about how make -j N works (without diving into academic naming), it’s essentially:

    Build the DAG by reading all rules and figuring out dependencies.

    Compute each node’s in‐degree (number of unmet prerequisites).

    Initialize a ready list with all in‐degree 0 targets.

    Event loop:

        Whenever a shell‐job slot is free (at most N jobs running simultaneously), pop the next ready target (often in topological or file‐appearance order) and launch it.

        When it finishes, decrement the in‐degree of its children; any child whose in‐degree hits zero is appended to the ready queue.

        Repeat until no ready targets remain and no jobs are running.

There is no static “longest‐path” priority by default; Make takes whichever ready rule comes next in its internal DFS/BFS ordering. That simplistic approach works well for many small compile tasks, but can cause pathological slowdown if a long‐running rule happens to get scheduled “too early” and blocks numerous short, independent rules.
Key Takeaways

    Exact optimal scheduling is NP-hard once you have arbitrary DAGs and heterogeneous durations.

    Greedy FIFO is trivial (and what Make uses), but often suboptimal.

    List scheduling with a good static priority (bottom‐level / critical path) is by far the most common heuristic in HPC and build engines once you can estimate task durations.

    If your tasks truly have the same weight, Coffman–Graham level scheduling can give you a guaranteed bound, but it’s less flexible in real‐world builds.

    On modern multicore machines, many runtimes embed work‐stealing to adapt to runtime variability, but that usually still relies on a bottom‐level or depth‐first policy to populate each worker’s deque.

In practice, if you’re implementing a Make‐like DAG scheduler with --jobs N, start with the simple FIFO scheme. If you later find you need better utilization, compute each rule’s “bottom‐level” (i.e. longest path of downstream tasks or approximate run time) and turn your ready queue into a max‐heap keyed by that bottom‐level. That single change (static critical‐path list scheduling) is by far the “next step” most systems take.
You said:
what does AirFlow use?
ChatGPT said:

Airflow’s scheduler is not a pure FIFO (Kahn-style) engine; rather, it uses a form of priority-driven (list) scheduling. In broad strokes, here’s what happens each “scheduling iteration”:

    Dependency Tracking

        The scheduler continuously parses all DAG files and—whenever a DAGRun becomes active—computes each task’s “readiness” by checking that all upstream dependencies have succeeded. Internally, this is exactly the same DAG traversal/in-degree bookkeeping you’d see in any make-style system: when a task’s in-degree (number of unmet prerequisites) drops to zero, that task is marked as ready.

    Priority Weights (“downstream” weight_rule)

        Every task/operator in Airflow gets a priority_weight (default = 1) and a weight_rule (default = “downstream”).

        The scheduler takes each task’s raw priority_weight and—if using the downstream rule—computes an effective weight by adding up the priority_weight of all tasks downstream of it (recursively). In other words:
        effective_weight(T)  =  priority_weight(T)  +  ∑T→S  effective_weight(S) ,
        effective_weight(T)=priority_weight(T)+T→S∑​effective_weight(S),

        so that tasks on the “longer” or “heavier” branch of the DAG naturally bubble up to higher priority.
        Apache Airflow

    Pools & Slot Counting

        Airflow enforces resource constraints by putting tasks into pools. Each pool has a fixed number of slots (e.g. CPU cores or arbitrary slot quotas). At scheduling time, the scheduler asks: “How many free slots remain in each pool?” and will schedule at most that many tasks from the ready set into each pool, per scheduling loop.
        Apache Airflow

    Priority Queue / List Scheduling

        Once all currently ready tasks have their effective weights computed (using the downstream rule), the scheduler keeps them in a max-heap (i.e. a priority queue) keyed by “effective weight.”

        Whenever a pool has free slots, the scheduler pops the highest-weight task from that pool’s ready queue and dispatches it to the chosen executor (e.g., CeleryExecutor, KubernetesExecutor, etc.).

        If two tasks have the same effective weight, they fall back to FIFO order (or, more precisely, the order in which Airflow discovered them in the last DAG parse).

    Iteration & Re-evaluation

        As soon as any task finishes (or fails), the scheduler revisits its children: each downstream task’s in-degree is decremented, and if it hits zero, that child is enqueued into the ready set (with its precomputed effective weight) for the next pass.

        Because Airflow’s default weight_rule=downstream was calculated at DAG-parse time, the scheduler does not normally recompute weights at runtime—i.e. it’s a semi-static list scheduler. If you adjust priority_weight by code (or via SLA misses, etc.), then on the next parse/reload the downstream sums get recalculated, but there is no fully dynamic “recompute bottom-levels on the fly” in the core scheduler.

Fundamental Aspects Summarized

    Offline/Static Weight Computation

        Before any task ever runs, Airflow traverses the DAG:

            Assign each node a base priority_weight (default = 1 unless overridden).

            Apply the weight_rule (by default “downstream”) to compute a single “effective” weight for each task:
            weff(t)  =  priority_weight(t)  +  ∑(t→s)weff(s).
            weff​(t)=priority_weight(t)+(t→s)∑​weff​(s).

            That means tasks at the “top” of a long chain of successors get higher weights.

    Ready-Queue List Scheduling

        At runtime, every task whose upstream dependencies are satisfied goes into a priority queue keyed by the precomputed w_eff(t).

        As soon as a pool has an open slot, the scheduler pops the highest-weight task and sends it to the executor. If multiple tasks share the same weight, they’re ordered in FIFO order of “when they became ready.”

    Pooling & Concurrency Control

        You define named pools (e.g., default_pool, big_data_pool, etc.), each with a fixed slot count.

        When the scheduler selects tasks to run, it never exceeds the available slots in any pool. In effect, each pool is treated as a bottleneck resource, and the scheduler simply asks “how many open slots are left?”—then dispatches up to that many highest-priority ready tasks per pool, per scheduling iteration.

    “As Soon As Possible” Bias

        Though tasks have priorities, Airflow still aims to push everything through quickly: if there are fewer ready tasks than open slots, it will simply schedule all of them, regardless of relative weight. Only when there is resource contention (i.e. more ready tasks than free slots) does the priority ordering actually matter.
        Apache Airflow

        In practice, this means:

            If you have 100 ready tasks in a pool of size 10, the scheduler picks the 10 tasks with the highest effective_weight.

            If you only have 3 ready tasks and 10 free slots, all 3 run immediately—priorities are irrelevant.

Why “Downstream” (Bottom-Level) Makes Sense for Airflow

    By defaulting to weight_rule='downstream', Airflow basically implements a static bottom-level (critical-path) list scheduler:

        A task with many downstream tasks (or “heavy” children) gets ranked higher, so the longest chain of dependent tasks is addressed first.

        This reduces the risk of one “long chain” of tasks stalling the DAG’s overall completion.

    It’s a balance between:

        Simplicity (only one offline pass to compute each node’s downstream sum), and

        Throughput (a ready queue that always picks the highest-weight tasks when slots are limited).

In a Nutshell

    Airflow’s scheduler = list scheduling with static “downstream” (bottom-level) weights + pool-slot enforcement + FIFO tie-breaker.

    Compared to a vanilla make -j N approach (which is pure FIFO), Airflow’s “priority_weight + downstream” rule is a simple way to approximate critical-path scheduling—ensuring that tasks on the longest suffix are favored whenever pools are oversubscribed.

Once you understand that, the high-level answer to “What does Airflow use?” is:

    Airflow uses a priority-driven (list) scheduler that orders ready tasks by their “downstream” weight (i.e. sum of own priority_weight plus all downstream priority_weights), and then dispatches them into pool slots in order of descending weight. If a pool has more free slots than ready tasks, it simply schedules everything; only when you have more ready tasks than slots does weight ordering actually pick winners.
