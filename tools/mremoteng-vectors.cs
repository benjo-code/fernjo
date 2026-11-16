#:package BouncyCastle.Cryptography@2.6.2
#:property PublishAot=false

// Independent encrypt-side port of the mRemoteNG 2.6 secret format, used to generate known-answer
// vectors for the Rust decryptor in crates/fernjo-import (so it is not only tested against itself).
//   dotnet run tools/mremoteng-vectors.cs -- crates/fernjo-import/tests/fixtures
// Writes <dir>/vectors.json. CI never runs this; the generated file is committed.
//
// Format: base64( salt[16] || nonce[16] || ciphertext || tag[16] ), AES-256-GCM with a 16-byte nonce and
// the salt as associated data; key = PBKDF2-HMAC-SHA1 over the UTF-8 password bytes, 32-byte output.

using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using Org.BouncyCastle.Crypto.Engines;
using Org.BouncyCastle.Crypto.Generators;
using Org.BouncyCastle.Crypto.Modes;
using Org.BouncyCastle.Crypto.Parameters;

var outDir = args.Length > 0 ? args[0] : "crates/fernjo-import/tests/fixtures";
Directory.CreateDirectory(outDir);

var cases = new (string Password, int Iterations, string Plaintext)[]
{
    ("mR3m", 1000, "Passw0rd!"),
    ("custom key 1", 5000, "s3cret-for-host"),
    ("pässwörd-密码-🔑", 1000, "Grüße, 世界 – 🔐"),
    ("mR3m", 1000, string.Concat(Enumerable.Range(0, 2000).Select(i => (char)('a' + i % 26)))),
};

var vectors = cases.Select(c => new Dictionary<string, object>
{
    ["password"] = c.Password,
    ["iterations"] = c.Iterations,
    ["plaintext"] = c.Plaintext,
    ["secret"] = Encrypt(c.Plaintext, c.Password, c.Iterations),
}).ToList();

var path = Path.Combine(outDir, "vectors.json");
File.WriteAllText(path, JsonSerializer.Serialize(vectors, new JsonSerializerOptions { WriteIndented = true }) + "\n");
Console.WriteLine($"wrote {vectors.Count} vectors to {path}");
return 0;

static string Encrypt(string plaintext, string password, int iterations)
{
    var salt = RandomNumberGenerator.GetBytes(16);
    var nonce = RandomNumberGenerator.GetBytes(16);

    var gen = new Pkcs5S2ParametersGenerator();
    gen.Init(Encoding.UTF8.GetBytes(password), salt, iterations);
    var key = ((KeyParameter)gen.GenerateDerivedMacParameters(256)).GetKey();

    var cipher = new GcmBlockCipher(new AesEngine());
    cipher.Init(true, new AeadParameters(new KeyParameter(key), 128, nonce, salt));
    var input = Encoding.UTF8.GetBytes(plaintext);
    var output = new byte[cipher.GetOutputSize(input.Length)];
    var len = cipher.ProcessBytes(input, 0, input.Length, output, 0);
    cipher.DoFinal(output, len);

    return Convert.ToBase64String(salt.Concat(nonce).Concat(output).ToArray());
}
