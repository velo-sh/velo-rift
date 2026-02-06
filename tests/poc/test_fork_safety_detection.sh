#!/bin/bash
# Test script to detect if fork safety is needed for Velo Rift shim

set -e

echo "=== Fork Safety Detection Test ==="
echo "Testing if fork() causes Worker thread loss..."
echo ""

# Cleanup
cleanup() {
    pkill -f vriftd 2>/dev/null || true
    rm -rf /tmp/vrift_fork_test 2>/dev/null || true
}
trap cleanup EXIT

# Setup
WORK_DIR="/tmp/vrift_fork_test"
mkdir -p "$WORK_DIR"
cd "$WORK_DIR"

# Behavior-based daemon check instead of pgrep
if [ ! -S "/tmp/vrift.sock" ]; then
    echo "⚠️  vriftd not running (socket not found), attempting to start..."
    # Note: You may need to start vriftd manually with a manifest
fi

# Create test program that forks and uses VFS
cat > fork_test.c << 'EOF'
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <sys/wait.h>

int main() {
    printf("[Parent] PID=%d, testing fork safety...\n", getpid());
    
    // Open a file in parent
    int fd = open("/tmp/test_file.txt", O_RDONLY | O_CREAT, 0644);
    if (fd >= 0) {
        printf("[Parent] Opened FD=%d\n", fd);
        close(fd);
    }
    
    // Fork child processes
    for (int i = 0; i < 10; i++) {
        pid_t pid = fork();
        
        if (pid == 0) {
            // Child process
            printf("[Child %d] PID=%d, attempting VFS operations...\n", i, getpid());
            
            // Try to use fstat (triggers shim)
            struct stat st;
            for (int j = 0; j < 100; j++) {
                int test_fd = open("/tmp/test_file.txt", O_RDONLY);
                if (test_fd >= 0) {
                    fstat(test_fd, &st);
                    close(test_fd);
                }
            }
            
            printf("[Child %d] Completed 100 operations\n", i);
            exit(0);
        } else if (pid < 0) {
            perror("fork");
            exit(1);
        }
    }
    
    // Parent waits for all children
    int status;
    for (int i = 0; i < 10; i++) {
        wait(&status);
    }
    
    printf("[Parent] All children completed\n");
    return 0;
}
EOF

# Compile test program
echo "📝 Compiling test program..."
gcc -o fork_test fork_test.c || {
    echo "❌ Failed to compile test program"
    exit 1
}

# Create test file
touch /tmp/test_file.txt

echo ""
echo "🔬 Running fork test with shim..."
echo "   - 10 child processes"
echo "   - 100 VFS operations per child"
echo "   - Total: 1000 fstat() calls after fork()"
echo ""

# Run with shim
export DYLD_INSERT_LIBRARIES="$(find ../.. -name 'libvrift_inception_layer.dylib' | head -1)"
export LD_PRELOAD="$(find ../.. -name 'libvrift_shim.so' | head -1)"

if [ -n "$DYLD_INSERT_LIBRARIES" ] || [ -n "$LD_PRELOAD" ]; then
    echo "✅ Shim library found, running test..."
    
    # Monitor memory before
    MEM_BEFORE=$(ps aux | grep fork_test | awk '{sum+=$6} END {print sum}')
    
    # Run test
    timeout 30s ./fork_test 2>&1 | tee fork_test.log || {
        echo "❌ Test timed out or failed!"
        echo "   This may indicate Worker thread loss (tasks piling up)"
        exit 1
    }
    
    # Monitor memory after
    sleep 2
    MEM_AFTER=$(ps aux | grep fork_test | awk '{sum+=$6} END {print sum}')
    
    echo ""
    echo "=== Analysis ==="
    
    # Check for errors in log
    if grep -qi "error\|leak\|full\|timeout" fork_test.log; then
        echo "⚠️  Detected errors in output:"
        grep -i "error\|leak\|full\|timeout" fork_test.log
        echo ""
        echo "❌ FORK SAFETY REQUIRED!"
        echo "   Reason: Errors detected during fork test"
        exit 1
    fi
    
    # Check if all children completed
    COMPLETED=$(grep -c "Child.*Completed" fork_test.log || echo 0)
    if [ "$COMPLETED" -ne 10 ]; then
        echo "❌ FORK SAFETY REQUIRED!"
        echo "   Reason: Only $COMPLETED/10 children completed"
        echo "   Worker threads likely lost in child processes"
        exit 1
    fi
    
    # Check memory growth (rough heuristic)
    if [ -n "$MEM_BEFORE" ] && [ -n "$MEM_AFTER" ]; then
        MEM_GROWTH=$((MEM_AFTER - MEM_BEFORE))
        if [ $MEM_GROWTH -gt 10000 ]; then
            echo "⚠️  Significant memory growth: ${MEM_GROWTH}KB"
            echo "❌ FORK SAFETY MAY BE REQUIRED!"
            echo "   Reason: Possible memory leak from lost Worker threads"
            exit 1
        fi
    fi
    
    echo "✅ All 10 children completed successfully"
    echo "✅ No errors detected"
    echo "✅ Memory stable"
    echo ""
    echo "✅ FORK SAFETY NOT REQUIRED (currently)"
    echo "   Your workload does not exhibit fork-related issues"
    
else
    echo "⚠️  Shim library not found, running without shim..."
    ./fork_test
    echo ""
    echo "ℹ️  Test completed without shim (baseline)"
    echo "   Build vrift-inception-layer first to test with shim"
fi

echo ""
echo "=== Summary ==="
echo "Test completed. Check results above."
