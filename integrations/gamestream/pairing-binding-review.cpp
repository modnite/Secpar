#include "secpar_pairing.h"
#include <cassert>
#include <fstream>
#include <iterator>
int main(int argc,char** argv) {
 assert(argc==3);
 std::ifstream input(argv[1]);std::string certificate((std::istreambuf_iterator<char>(input)),{});
 std::string hash=argv[2];
 assert(secpar_pairing::certificate_sha256(certificate)==hash);
 assert(secpar_pairing::matches("owner-device",hash,"owner-device",certificate));
 assert(!secpar_pairing::matches("another-device",hash,"owner-device",certificate));
 std::string wrong=hash;wrong[0]=wrong[0]=='0'?'1':'0';
 assert(!secpar_pairing::matches("owner-device",wrong,"owner-device",certificate));
 assert(!secpar_pairing::matches("",hash,"owner-device",certificate));
 assert(!secpar_pairing::matches("owner-device","","owner-device",certificate));
 assert(!secpar_pairing::matches("owner-device",hash,"owner-device","invalid certificate"));
 assert(!secpar_pairing::matches(std::string("id\0x",4),hash,"owner-device",certificate));
 assert(secpar_pairing::matches("","","legacy",certificate));
 std::string crlf;for(char c:certificate){if(c=='\n')crlf+='\r';crlf+=c;}
 assert(secpar_pairing::certificate_sha256(crlf)==hash);
 assert(secpar_pairing::certificate_sha256(std::string(16385,'x')).empty());
}
