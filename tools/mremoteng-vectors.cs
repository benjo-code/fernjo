#:package BouncyCastle.Cryptography@2.6.2
#:property PublishAot=false

// Independent encrypt-side port of the mRemoteNG 2.6 secret format, used to generate known-answer
// vectors for the Rust decryptor in crates/fernjo-import (so it is not only tested against itself).
//   dotnet run tools/mremoteng-vectors.cs -- crates/fernjo-import/tests/fixtures
// Writes <dir>/vectors.json plus whole confCons.xml fixtures (default-key.xml, custom-key.xml, full-file.xml)
// with a ConfVersion 2.6 attribute set and, next to each, <name>.expected.json (the tree the Rust reader must
// produce; the passwords are synthetic and in clear). CI never runs this; the generated files are committed.
//
// Format: base64( salt[16] || nonce[16] || ciphertext || tag[16] ), AES-256-GCM with a 16-byte nonce and
// the salt as associated data; key = PBKDF2-HMAC-SHA1 over the UTF-8 password bytes, 32-byte output.

using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Xml.Linq;
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

// ---- whole-file fixtures ----------------------------------------------------------------------------------
var tree = new List<N>
{
    new("Container", "Servers", "All the servers", "", "", 3389, "admin", "CORP", "folder-pw-1", false, false, false, false, "", new List<N>
    {
        new("Connection", "web1", "Front <web> & \"proxy\"", "web1.example.com", "SSH2", 22, "root", "", "p@ss<&>\"'\\", false, false, false, false, "Default Settings", new()),
        new("Connection", "rdp1", "", "10.0.0.5", "RDP", 3389, "", "", "", true, true, true, true, "", new()),
        new("Container", "Nested", "", "", "", 3389, "", "", "", true, true, true, true, "", new List<N>
        {
            new("Connection", "db", "database", "db.example.com", "SSH2", 2222, "dbadmin", "EXAMPLE", "", false, true, false, false, "MySession", new()),
            new("Connection", "vnc1", "", "10.0.0.9", "VNC", 5900, "", "", "vnc-pw", false, false, false, false, "", new()),
        }),
    }),
    new("Connection", "jump", "", "jump.example.com", "SSH2", 22, "jumper", "", "Grüße, 世界 – 🔐", false, false, false, false, "", new()),
};

var files = new (string Name, string Key, int Iterations, bool FullFile)[]
{
    ("default-key", "mR3m", 1000, false),
    ("custom-key", "fixture-key-ünï", 5000, false),
    ("full-file", "fixture-key", 1000, true),
};
foreach (var f in files)
{
    File.WriteAllText(Path.Combine(outDir, f.Name + ".xml"), BuildFile(tree, f.Key, f.Iterations, f.FullFile), new UTF8Encoding(false));
    File.WriteAllText(Path.Combine(outDir, f.Name + ".expected.json"),
        JsonSerializer.Serialize(tree.Select(Expected).ToList(), new JsonSerializerOptions { WriteIndented = true }) + "\n");
    Console.WriteLine($"wrote fixture {f.Name}");
}
return 0;

static string BuildFile(List<N> tree, string key, int iterations, bool fullFile)
{
    var root = new XElement("Connections",
        new XAttribute("Name", "Connections"),
        new XAttribute("Export", "false"),
        new XAttribute("EncryptionEngine", "AES"),
        new XAttribute("BlockCipherMode", "GCM"),
        new XAttribute("KdfIterations", iterations),
        new XAttribute("FullFileEncryption", fullFile ? "true" : "false"),
        new XAttribute("Protected", Encrypt("ThisIsNotProtected", key, iterations)),
        new XAttribute("ConfVersion", "2.6"));
    var nodes = tree.Select(n => ToXml(n, key, iterations)).ToList();
    if (fullFile)
    {
        var inner = string.Concat(nodes.Select(n => n.ToString(SaveOptions.DisableFormatting)));
        root.Add(new XText(Encrypt(inner, key, iterations)));
    }
    else root.Add(nodes);
    return "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n" + root.ToString();
}

static XElement ToXml(N n, string key, int iterations)
{
    string B(bool b) => b ? "true" : "false";
    var e = new XElement("Node",
        new XAttribute("Name", n.Name), new XAttribute("Type", n.Kind), new XAttribute("Descr", n.Descr),
        new XAttribute("Icon", "mRemoteNG"), new XAttribute("Panel", "General"),
        new XAttribute("Id", Guid.NewGuid().ToString()),
        new XAttribute("Username", n.User), new XAttribute("Domain", n.Domain),
        new XAttribute("Password", n.Password.Length == 0 ? "" : Encrypt(n.Password, key, iterations)),
        new XAttribute("Hostname", n.Host), new XAttribute("Protocol", n.Protocol),
        new XAttribute("PuttySession", n.Putty), new XAttribute("Port", n.Port),
        new XAttribute("ConnectToConsole", "false"), new XAttribute("UseCredSsp", "true"),
        new XAttribute("RenderingEngine", "IE"), new XAttribute("ICAEncryptionStrength", "EncrBasic"),
        new XAttribute("RDPAuthenticationLevel", "NoAuth"), new XAttribute("RDPMinutesToIdleTimeout", "0"),
        new XAttribute("RDPAlertIdleTimeout", "false"), new XAttribute("LoadBalanceInfo", ""),
        new XAttribute("Colors", "Colors16Bit"), new XAttribute("Resolution", "FitToWindow"),
        new XAttribute("AutomaticResize", "true"), new XAttribute("DisplayWallpaper", "false"),
        new XAttribute("DisplayThemes", "false"), new XAttribute("EnableFontSmoothing", "false"),
        new XAttribute("EnableDesktopComposition", "false"), new XAttribute("CacheBitmaps", "false"),
        new XAttribute("RedirectDiskDrives", "false"), new XAttribute("RedirectPorts", "false"),
        new XAttribute("RedirectPrinters", "false"), new XAttribute("RedirectSmartCards", "false"),
        new XAttribute("RedirectSound", "DoNotPlay"), new XAttribute("SoundQuality", "Dynamic"),
        new XAttribute("RedirectKeys", "false"), new XAttribute("Connected", "false"),
        new XAttribute("PreExtApp", ""), new XAttribute("PostExtApp", ""), new XAttribute("MacAddress", ""),
        new XAttribute("UserField", ""), new XAttribute("ExtApp", ""),
        new XAttribute("VNCCompression", "CompNone"), new XAttribute("VNCEncoding", "EncHextile"),
        new XAttribute("VNCAuthMode", "AuthVNC"), new XAttribute("VNCProxyType", "ProxyNone"),
        new XAttribute("VNCProxyIP", ""), new XAttribute("VNCProxyPort", "0"),
        new XAttribute("VNCProxyUsername", ""), new XAttribute("VNCProxyPassword", ""),
        new XAttribute("VNCColors", "ColNormal"), new XAttribute("VNCSmartSizeMode", "SmartSNo"),
        new XAttribute("VNCViewOnly", "false"), new XAttribute("RDGatewayUsageMethod", "Never"),
        new XAttribute("RDGatewayHostname", ""), new XAttribute("RDGatewayUseConnectionCredentials", "Yes"),
        new XAttribute("RDGatewayUsername", ""), new XAttribute("RDGatewayPassword", ""),
        new XAttribute("RDGatewayDomain", ""),
        new XAttribute("InheritCacheBitmaps", "false"), new XAttribute("InheritColors", "false"),
        new XAttribute("InheritDescription", "false"), new XAttribute("InheritDisplayThemes", "false"),
        new XAttribute("InheritDisplayWallpaper", "false"), new XAttribute("InheritDomain", B(n.IDomain)),
        new XAttribute("InheritIcon", "false"), new XAttribute("InheritPanel", "false"),
        new XAttribute("InheritPassword", B(n.IPass)), new XAttribute("InheritPort", B(n.IPort)),
        new XAttribute("InheritProtocol", "false"), new XAttribute("InheritPuttySession", "false"),
        new XAttribute("InheritRedirectDiskDrives", "false"), new XAttribute("InheritResolution", "false"),
        new XAttribute("InheritUseConsoleSession", "false"), new XAttribute("InheritUseCredSsp", "false"),
        new XAttribute("InheritUsername", B(n.IUser)));
    if (n.Kind == "Container") { e.SetAttributeValue("Expanded", "true"); e.SetAttributeValue("Favorite", "false"); }
    e.Add(n.Children.Select(c => ToXml(c, key, iterations)));
    return e;
}

static Dictionary<string, object?> Expected(N n) => new()
{
    ["kind"] = n.Kind == "Container" ? "container" : "connection",
    ["name"] = n.Name,
    ["description"] = n.Descr,
    ["hostname"] = n.Host,
    ["protocol"] = n.Protocol,
    ["port"] = n.Port,
    ["username"] = n.User,
    ["domain"] = n.Domain,
    ["password"] = n.Password.Length == 0 ? null : n.Password,
    ["passwordError"] = false,
    ["inherit"] = new Dictionary<string, bool>
        { ["username"] = n.IUser, ["password"] = n.IPass, ["domain"] = n.IDomain, ["port"] = n.IPort },
    ["puttySession"] = n.Putty,
    ["children"] = n.Children.Select(Expected).ToList(),
};

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

record N(string Kind, string Name, string Descr, string Host, string Protocol, int Port, string User, string Domain,
    string Password, bool IUser, bool IPass, bool IDomain, bool IPort, string Putty, List<N> Children);
