ifeq ($(RELEASE), 1)
modtarget=target/release/libpolarsignals_module.so
else
modtarget=target/debug/libpolarsignals_module.so
endif

.PHONY : symlink target/debug/libpolarsignals_module.so target/release/libpolarsignals_module.so
symlink : $(modtarget)
	rm -f polarsignals-module.so
	ln -sr $(modtarget) polarsignals-module.so

target/debug/libpolarsignals_module.so :
	cargo build	
	readelf -SW target/debug/libpolarsignals_module.so | grep -q '\.debug_gdb_scripts' || objcopy --add-section .debug_gdb_scripts=debug_gdb_scripts.bin target/debug/libpolarsignals_module.so

target/release/libpolarsignals_module.so :
	cargo build --release
	readelf -SW target/release/libpolarsignals_module.so | grep -q '\.debug_gdb_scripts' || objcopy --add-section .debug_gdb_scripts=debug_gdb_scripts.bin target/release/libpolarsignals_module.so

.PHONY : clean
clean :
	cargo clean
	rm -f polarsignals-module.so
