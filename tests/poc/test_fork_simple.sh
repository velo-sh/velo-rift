#!/bin/bash
# Simplified Fork Safety Detection for Velo Rift
# Tests if fork() without exec() causes Worker thread loss

set -e

echo "=== Simplified Fork Safety Test ==="
echo ""

# Find shim library
SHIM_LIB=$(find target/release -name 'libvrift_inception_layer.dylib' -o -name 'libvrift_shim.so' 2>/dev/null | head -1)

if [ -z "$SHIM_LIB" ]; then
    echo "❌ Shim library not found!"
    echo "   Run: cargo build -p vrift-inception-layer --release"
    exit 1
fi

echo "✅ Found shim: $SHIM_LIB"
echo ""

# Create simple test program
cat > /tmp/fork_test_simple.c << 'EOF'
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <sys/stat.h>
#include <sys/wait.h>

int main() {
    // Create test file
    FILE *f = fopen("/tmp/vrift_test.txt", "w");
    if (f) {
        fprintf(f, "test");
        fclose(f);
    }
    
    printf("Parent: Forking 5 children that will NOT exec()...\n");
    
    for (int i = 0; i < 5; i++) {
        pid_t pid = fork();
        
        if (pid == 0) {
            // Child: continue using VFS WITHOUT exec()
            printf("Child %d: Doing 50 fstat calls...\n", i);
            
            struct stat st;
            for (int j = 0; j < 50; j++) {
                if (stat("/tmp/vrift_test.txt", &st) != 0) {
                    printf("Child %d: ERROR on stat #%d\n", i, j);
                }
            }
            
            printf("Child %d: DONE\n", i);
            exit(0);
        }
    }
    
    // Wait for all children
    for (int i = 0; i < 5; i++) {
        int status;
        wait(&status);
    }
    
    printf("Parent: All children completed\n");
    return 0;
}
EOF

echo "📝 Compiling test program..."
gcc -o /tmp/fork_test_simple /tmp/fork_test_simple.c

echo "🔬 Test 1: Running WITHOUT shim (baseline)..."
/tmp/fork_test_simple
echo ""

echo "🔬 Test 2: Running WITH shim (fork safety test)..."
if [[ "$OSTYPE" == "darwin"* ]]; then
    DYLD_INSERT_LIBRARIES="$PWD/$SHIM_LIB" /tmp/fork_test_simple
else
    LD_PRELOAD="$PWD/$SHIM_LIB" /tmp/fork_test_simple
fi

echo ""
echo "=== Analysis ==="
echo ""
echo "Did all 5 children complete?"
echo "  - YES: ✅ Fork safety NOT needed (exec() always called, or fast-path works)"
echo "  - NO:  ❌ Fork safety REQUIRED (Worker threads lost)"
echo ""
echo "Check output above for 'Child X: DONE' messages."
echo "Expected: 5 'DONE' messages"
