cp /bin/ls ./ls_no_sip
codesign --force --sign - ./ls_no_sip
export DYLD_INSERT_LIBRARIES=$(pwd)/target/release/libvrift_shim.dylib
export DYLD_FORCE_FLAT_NAMESPACE=1
export VRIFT_MANIFEST=$(pwd)/project_a.manifest
export VR_THE_SOURCE=$(pwd)/the_source
export VRIFT_VFS_PREFIX="/vrift"
export VRIFT_DEBUG=1
export VRIFT_DISABLE_MMAP=1
echo "Running ls_no_sip..."
./ls_no_sip -d /vrift
