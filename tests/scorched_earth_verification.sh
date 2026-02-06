#!/bin/bash
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${YELLOW}🔥 Starting Scorched Earth Verification...${NC}"

# 1. Aggressive Cleanup
echo "💀 Killing all existing vrift processes..."
pkill -9 -f vriftd || true
pkill -9 -f vrift || true
pkill -9 -f repro_rwlock_stress || true
sleep 2

# 2. Check for Zombies
echo "🧟 Checking for Undead Processes (Zombies/UE)..."
ZOMBIES=$(ps aux | grep -E "vrift|repro" | grep -v grep | grep -E " U | D ")
if [ -n "$ZOMBIES" ]; then
    echo -e "${RED}CRITICAL WARNING: Found Uninterruptible (Zombie) Processes:${NC}"
    echo "$ZOMBIES"
    echo -e "${RED}These processes CANNOT be killed by user or root. They hold kernel locks.${NC}"
    echo -e "${YELLOW}Proceeding anyway to demonstrate the effect...${NC}"
else
    echo -e "${GREEN}No zombie processes found! (Unexpected but good)${NC}"
fi

# 3. Clean Filesystem
echo "🧹 Cleaning temporary files..."
rm -rf /tmp/vrift* 2>/dev/null
rm -f /tmp/vriftd.sock 2>/dev/null

# 4. Run Isolation Test
echo "🛡️ Launching Independent Isolation Environment..."
./tests/run_emergent_isolation.sh
