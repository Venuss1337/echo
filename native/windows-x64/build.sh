#!/usr/bin/env bash
# Run in an isolated MSYS2 UCRT64 shell with independently verified inputs.
set -euo pipefail

root=${1:?absolute input and build directory required}
jobs=${2:-4}
profile=${3:-feasibility}
case "$profile" in
    feasibility) io_flags=(--enable-schannel --disable-everything --enable-protocol=file,http,https,tcp,tls,httpproxy)
                 demuxers=aac,flac,matroska,mov,mp3,ogg,wav,wv ;;
    local)       io_flags=(--disable-network --disable-everything --disable-protocols)
                 demuxers=aac,flac,mov,mp3,ogg,wav ;;
    *)           echo "Unknown native profile: $profile" >&2; exit 2 ;;
esac
[[ $root = /* && ${MSYSTEM:-} = UCRT64 && $(gcc -dumpmachine) = x86_64-w64-mingw32 ]]
cd "$root"

mpv=mpv-b0841e8c4d156b8403dc1876c4cbd9011eb583f3
sha256sum --strict -c <<'CHECKSUMS'
1a4d394ae63d190a526cd052eb4415c156b5c5e2df37efab41a7de0b8fde2433  ffmpeg-9.1.tar.xz
a4a79fa6ca728762ac407734b0f93dbcbba250c3bb3bf97b77a342042be802e4  mpv.tar.gz
3bf5d72786682c194387c33d563872f6bfe202a9816f9dde6535a31a9b2e0d74  libplacebo-v7.387.0.tar.gz
CHECKSUMS

# Fresh build directories keep previous configure results out of the candidate.
mkdir build
tar -xf ffmpeg-9.1.tar.xz -C build
tar -xf mpv.tar.gz -C build
tar -xf libplacebo-v7.387.0.tar.gz -C build
cd build
# FFmpeg embeds its configure arguments. A fixed logical prefix inside each
# isolated MSYS2 installation keeps host work-directory paths out of the DLL.
prefix=/echo-native
[[ ! -e $prefix ]]
export SOURCE_DATE_EPOCH=1795305600
export LC_ALL=C TZ=UTC
export PKG_CONFIG_LIBDIR="$prefix/lib/pkgconfig:/ucrt64/lib/pkgconfig"
unset PKG_CONFIG_PATH

cd ffmpeg-9.1
./configure --prefix="$prefix" --arch=x86_64 --target-os=mingw32 \
    --cc=gcc --cxx=g++ --pkg-config=pkgconf --pkg-config-flags=--static \
    --disable-autodetect --disable-programs --disable-doc --disable-debug \
    --enable-static --disable-shared --disable-avdevice \
    --enable-gpl --enable-version3 \
    "${io_flags[@]}" --enable-demuxer="$demuxers" \
    --enable-parser=aac,aac_latm,flac,mpegaudio,opus,vorbis \
    --enable-decoder=aac,aac_latm,alac,flac,mp3,mp3float,opus,vorbis,wavpack,pcm_alaw,pcm_mulaw,pcm_u8,pcm_s16le,pcm_s24le,pcm_s32le,pcm_f32le,pcm_f64le \
    --enable-filter=abuffer,abuffersink,aformat,anull,aresample,atempo,volume \
    --extra-cflags='-fstack-protector-strong -D_FORTIFY_SOURCE=2' \
    --extra-ldflags='-Wl,--dynamicbase,--nxcompat,--high-entropy-va'
make -j "$jobs"
make install
cd ..

if [[ $profile == local ]]; then
    # The host supplies primary bytes through stream_cb. Remove every native
    # stream opener (including OS file and fd handlers), and the indirect,
    # playlist, and other mpv demuxers. FFmpeg can only consume custom AVIO.
    sed -i '/^static const stream_info_t \*const stream_list\[\] = {$/,/^};$/c\static const stream_info_t *const stream_list[] = {\n    \&stream_info_cb,\n};' "$mpv/stream/stream.c"
    sed -i '/^static const demuxer_desc_t \*const demuxer_list\[\] = {$/,/^};$/c\static const demuxer_desc_t *const demuxer_list[] = {\n    \&demuxer_desc_lavf,\n    NULL,\n};' "$mpv/demux/demux.c"
    [[ $(sed -n '/^static const stream_info_t \*const stream_list\[\] = {$/,/^};$/p' "$mpv/stream/stream.c" | sed -n '2p') == '    &stream_info_cb,' ]]
    [[ $(sed -n '/^static const demuxer_desc_t \*const demuxer_list\[\] = {$/,/^};$/p' "$mpv/demux/demux.c" | sed -n '2p') == '    &demuxer_desc_lavf,' ]]
fi

# mpv requires libplacebo even for audio. Disable its optional video backends
# and Dolby Vision path so no libdovi or Rust runtime enters the static link.
meson setup placebo-build libplacebo-v7.387.0 --prefix="$(cygpath -m "$prefix")" \
    --wrap-mode=nodownload --buildtype=release --default-library=static \
    -Dauto_features=disabled -Ddovi=disabled -Dlibdovi=disabled -Ddemos=false \
    -Dc_args='-fstack-protector-strong -D_FORTIFY_SOURCE=2'
meson compile -C placebo-build -j "$jobs"
meson install -C placebo-build --no-rebuild

# Schannel provides TLS for the FFmpeg transport selected when libcurl is off.
# Only Wasapi is enabled among the optional platform backends.
# The MSYS2 static libass closure contains C++ objects. Put the
# matching GCC runtime in Meson's dependency group, after the users of it.
sed -i "/^dependencies += cc.find_library('atomic', required: false)$/a dependencies += cc.find_library('stdc++', static: true)" "$mpv/meson.build"
grep -Fq "dependencies += cc.find_library('stdc++', static: true)" "$mpv/meson.build"
# This header embeds __FILE__ via assertions in the mpv translation units.
# A logical filename keeps the locked header's diagnostic text portable.
placebo_header="$prefix/include/libplacebo/utils/libav_internal.h"
sed -i '1i #line 1 "libplacebo/utils/libav_internal.h"' "$placebo_header"
meson setup mpv-build "$mpv" --wrap-mode=nodownload --buildtype=release \
    -Dprefer_static=true \
    -Dauto_features=disabled -Dbuild-date=false -Dgpl=true -Dcplayer=false \
    -Dlibmpv=true -Dlua=disabled -Dgl=disabled -Dwasapi=enabled -Dwin32-threads=enabled \
    -Dc_args='-fstack-protector-strong -D_FORTIFY_SOURCE=2' \
    -Dc_link_args='-Wl,--dynamicbase,--nxcompat,--high-entropy-va,--no-insert-timestamp,-Map,libmpv-link.map'
meson compile -C mpv-build -j "$jobs"
