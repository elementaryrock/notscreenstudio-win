import Foundation
import Metal
import NSSCore

/// Separable Gaussian blur for image backgrounds, on the GPU, in linear light.
///
/// The blur radius is relative to the image (`BackgroundBlur.sigma`), not the output, so the same blurred
/// texture serves every output size and the preview and the export match. To stay cheap at large radii the
/// blur reads a smaller mip level of the source (each level halves sigma in texels), keeping the kernel at
/// most ~19 taps per pass. The result is a mipmapped `rgba16Float` texture holding linear-light colour,
/// which the main shader samples exactly like the sRGB image texture.
enum ImageBlur {
    /// The kernel works at a mip level where sigma is at most this many texels.
    static let maxWorkingSigma = 6.0
    /// Below this sigma (in full-resolution texels) blurring is skipped.
    static let minSigma = 0.3

    static let source = """
    #include <metal_stdlib>
    using namespace metal;

    // p: dir.x, dir.y, sigma (texels), radius (taps each side). level: source mip level to read.
    kernel void nss_blur(texture2d<float, access::read> src [[texture(0)]],
                         texture2d<float, access::write> dst [[texture(1)]],
                         constant float4& p [[buffer(0)]],
                         constant uint& level [[buffer(1)]],
                         uint2 gid [[thread_position_in_grid]]) {
        uint w = dst.get_width(), h = dst.get_height();
        if (gid.x >= w || gid.y >= h) { return; }
        int2 dir = int2(p.xy);
        int r = int(p.w);
        float k = -0.5 / (p.z * p.z);
        float4 acc = 0.0;
        float wsum = 0.0;
        int2 hi = int2(int(w) - 1, int(h) - 1);
        for (int i = -r; i <= r; i++) {
            int2 q = clamp(int2(gid) + dir * i, int2(0), hi);
            float wt = exp(float(i * i) * k);
            acc += src.read(uint2(q), level) * wt;
            wsum += wt;
        }
        dst.write(acc / wsum, gid);
    }
    """

    /// Mip level to blur at and the sigma (in that level's texels) for a full-resolution `sigma`.
    static func plan(sigma: Double, mipLevels: Int) -> (level: Int, sigma: Double) {
        guard sigma.isFinite, sigma > maxWorkingSigma else { return (0, max(sigma, 0)) }
        let level = min(max(Int(ceil(log2(sigma / maxWorkingSigma))), 0), max(mipLevels - 1, 0))
        return (level, sigma / Double(1 << level))
    }

    /// Blurs `image` (an sRGB texture with mips) by `amount` (0...1). Returns nil when no blur is needed.
    /// Synchronous: waits for the GPU. Called once per (image, amount), never per frame.
    static func blur(_ image: MTLTexture, amount: Double, device: MTLDevice, queue: MTLCommandQueue) throws -> MTLTexture? {
        let full = BackgroundBlur.sigma(amount: amount, shortSide: min(image.width, image.height))
        guard full >= minSigma else { return nil }
        let (level, sigma) = plan(sigma: full, mipLevels: image.mipmapLevelCount)
        let w = max(image.width >> level, 1), h = max(image.height >> level, 1)
        let pipeline = try PipelineCache.shared.blurPipeline(for: device)

        let tmpDesc = MTLTextureDescriptor.texture2DDescriptor(pixelFormat: .rgba16Float, width: w, height: h, mipmapped: false)
        tmpDesc.usage = [.shaderRead, .shaderWrite]
        tmpDesc.storageMode = .private
        let outDesc = MTLTextureDescriptor.texture2DDescriptor(pixelFormat: .rgba16Float, width: w, height: h, mipmapped: true)
        outDesc.usage = [.shaderRead, .shaderWrite]
        outDesc.storageMode = .private
        guard let tmp = device.makeTexture(descriptor: tmpDesc), let out = device.makeTexture(descriptor: outDesc),
            let cmd = queue.makeCommandBuffer(), let enc = cmd.makeComputeCommandEncoder()
        else { throw NSSError.renderFailed("blur texture allocation failed") }

        let radius = Float(max(1, Int((3 * sigma).rounded(.up))))
        let tg = MTLSize(width: 16, height: 16, depth: 1)
        let grid = MTLSize(width: (w + 15) / 16, height: (h + 15) / 16, depth: 1)
        enc.setComputePipelineState(pipeline)
        // Horizontal: source mip `level` -> tmp. Vertical: tmp (level 0) -> out.
        for (src, dst, dir, lvl) in [(image, tmp, SIMD2<Float>(1, 0), UInt32(level)), (tmp, out, SIMD2<Float>(0, 1), 0)] {
            var params = SIMD4<Float>(dir.x, dir.y, Float(sigma), radius)
            var lv = lvl
            enc.setTexture(src, index: 0)
            enc.setTexture(dst, index: 1)
            enc.setBytes(&params, length: MemoryLayout<SIMD4<Float>>.size, index: 0)
            enc.setBytes(&lv, length: MemoryLayout<UInt32>.size, index: 1)
            enc.dispatchThreadgroups(grid, threadsPerThreadgroup: tg)
        }
        enc.endEncoding()
        if out.mipmapLevelCount > 1, let blit = cmd.makeBlitCommandEncoder() {
            blit.generateMipmaps(for: out)
            blit.endEncoding()
        }
        cmd.commit()
        cmd.waitUntilCompleted()
        if let error = cmd.error { throw NSSError.renderFailed("blur GPU error: \(error.localizedDescription)") }
        return out
    }
}
